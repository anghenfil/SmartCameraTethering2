use std::io::BufReader;
use std::sync::Arc;
use gphoto2::camera::CameraEvent;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio::net::TcpStream;
use rustls::ClientConfig;
use rustls::RootCertStore;
use rustls::pki_types::ServerName;
use SmartCameraTethering2_shared_types::{MessageToCameraServer, MessageToPostProcessor, PostProcessingConfig};
use crate::camera::worker::{CameraConnection, CameraError};
use crate::websocket::scheme::{Image, ImageSource, MessageToClient};

const SERVER_HOST: &str = "localhost";
const SERVER_PORT: u16 = 9000;
const CERT_PATH: &str = "certs/client2.crt";
const KEY_PATH: &str = "certs/client2.key";
const CA_PATH: &str = "certs/root.crt";

#[derive(Debug, Clone)]
struct CameraFilePath {
    pub folder: String,
    pub file: String,
}

impl From<&gphoto2::file::CameraFilePath> for CameraFilePath {
    fn from(value: &gphoto2::file::CameraFilePath) -> Self {
        Self {
            folder: value.folder().to_string(),
            file: value.name().to_string(),
        }
    }
}

pub fn start_post_processing_worker(
    event_channel: tokio::sync::broadcast::Sender<Arc<CameraEvent>>,
    mut connection_channel: tokio::sync::broadcast::Receiver<CameraConnection>,
    msg2client_sender: tokio::sync::broadcast::Sender<MessageToClient>,
) -> tokio::sync::mpsc::Sender<Vec<PostProcessingConfig>> {
    let (sender, mut recv) = tokio::sync::mpsc::channel::<Vec<PostProcessingConfig>>(3);

    tokio::task::spawn(async move {
        let mut config: Vec<PostProcessingConfig> = vec![];
        let mut connection: Option<CameraConnection> = None;

        let tls_connector = match build_tls_connector() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to build TLS connector: {:?}", e);
                return;
            }
        };

        // Persistent write half of the TLS connection, shared across tasks
        let write_half: Arc<Mutex<Option<tokio::io::WriteHalf<TlsStream<TcpStream>>>>> =
            Arc::new(Mutex::new(None));

        // Shared session_id — set by response_reader when server sends SessionId
        let session_id: Arc<Mutex<Option<u64>>> = Arc::new(Mutex::new(None));

        // Channel to signal that the reader task detected a disconnect
        let (disconnect_tx, mut disconnect_rx) = tokio::sync::mpsc::channel::<()>(1);

        let mut event_channel_receiver = event_channel.subscribe();
        loop {
            tokio::select! {
                new_config = recv.recv() => {
                    match new_config {
                        Some(new_config) => {
                            println!("Received new post processing config: {:?}", new_config);
                            config = new_config;
                            let mut w = write_half.lock().await;
                            if let Some(ref mut writer) = *w {
                                if let Err(e) = send_framed(writer, &MessageToPostProcessor::SetPostProcessorConfig(config.clone())).await {
                                        eprintln!("Failed to send config to post-processing server: {:?}", e);
                                    }
                            }
                        }
                        None => {
                            eprintln!("Channel to post processing worker closed.");
                            return;
                        }
                    }
                },
                event = event_channel_receiver.recv() => {
                    match event {
                        Ok(event) => {
                            if let CameraEvent::NewFile(new_file) = event.as_ref() {
                                let file_path: CameraFilePath = new_file.into();
                                let name_lower = file_path.file.to_lowercase();

                                if name_lower.ends_with(".jpg") || name_lower.ends_with(".jpeg") {
                                    // JPG: download and send directly to client
                                    if let Some(conn) = &mut connection {
                                        match conn.camera.fs().download(&file_path.folder, &file_path.file).await {
                                            Ok(img) => match img.get_data(&conn.context).await {
                                                Ok(data) => {
                                                    let image = Image::new(data.to_vec(), "image/jpeg".to_string(), ImageSource::Capture);
                                                    let _ = msg2client_sender.send(MessageToClient::Image(image));
                                                }
                                                Err(e) => {
                                                    let _ = msg2client_sender.send(MessageToClient::CameraError(e.into()));
                                                }
                                            },
                                            Err(e) => {
                                                let _ = msg2client_sender.send(MessageToClient::CameraError(e.into()));
                                            }
                                        }
                                    }
                                } else {
                                    // RAW: send to post-processing server in a background task
                                    // so the select! loop is not blocked during the transfer
                                    if let Some(conn) = &connection {
                                        let conn = conn.clone();
                                        let file_path = file_path.clone();
                                        let write_half = write_half.clone();
                                        let session_id = session_id.clone();
                                        let config = config.clone();
                                        let msg2client_sender = msg2client_sender.clone();
                                        let disconnect_tx = disconnect_tx.clone();
                                        let tls_connector = tls_connector.clone();

                                        tokio::task::spawn(async move {
                                            let mut last_err = None;
                                            for attempt in 1..=3 {
                                                let mut w = write_half.lock().await;
                                                if let Some(ref mut writer) = *w {
                                                    match send_raw_to_server(writer, &conn, &file_path).await {
                                                        Ok(_) => {
                                                            last_err = None;
                                                            break;
                                                        }
                                                        Err(e) => {
                                                            eprintln!("Error sending raw to post-processing server (attempt {}/3): {:?}", attempt, e);
                                                            last_err = Some(e);
                                                            *w = None;
                                                        }
                                                    }
                                                } else {
                                                    break;
                                                }
                                                drop(w);
                                                if attempt < 3 {
                                                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                                                    let sid = *session_id.lock().await;
                                                    let is_resume = sid.is_some();
                                                    establish_connection(
                                                        &tls_connector,
                                                        &write_half,
                                                        &session_id,
                                                        &config,
                                                        &msg2client_sender,
                                                        disconnect_tx.clone(),
                                                        is_resume,
                                                    ).await;
                                                }
                                            }
                                            if let Some(e) = last_err {
                                                let _ = msg2client_sender.send(MessageToClient::CameraError(e));
                                                let _ = disconnect_tx.send(()).await;
                                            }
                                        });
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Error receiving camera event: {:?}", e);
                            return;
                        }
                    }
                },
                new_connection = connection_channel.recv() => {
                    match new_connection {
                        Ok(new_connection) => {
                            connection = Some(new_connection);
                            // Establish a fresh TLS connection (StartSession — new camera connection)
                            *session_id.lock().await = None;
                            establish_connection(
                                &tls_connector,
                                &write_half,
                                &session_id,
                                &config,
                                &msg2client_sender,
                                disconnect_tx.clone(),
                                false,
                            ).await;
                        }
                        Err(e) => {
                            eprintln!("Channel to post processing worker closed. {e}");
                        }
                    }
                },
                _ = disconnect_rx.recv() => {
                    eprintln!("Post-processing server disconnected, attempting reconnect...");
                    // Retry reconnect with backoff
                    let mut delay_secs = 1u64;
                    loop {
                        tokio::time::sleep(tokio::time::Duration::from_secs(delay_secs)).await;
                        let sid = *session_id.lock().await;
                        let is_resume = sid.is_some();
                        let connected = establish_connection(
                            &tls_connector,
                            &write_half,
                            &session_id,
                            &config,
                            &msg2client_sender,
                            disconnect_tx.clone(),
                            is_resume,
                        ).await;
                        if connected {
                            println!("Reconnected to post-processing server.");
                            break;
                        }
                        delay_secs = (delay_secs * 2).min(30);
                        eprintln!("Reconnect failed, retrying in {}s...", delay_secs);
                    }
                }
            }
        }
    });

    sender
}

/// Establishes a TLS connection, sends StartSession or ResumeSession, sends current config,
/// spawns a response reader. Returns true on success.
async fn establish_connection(
    tls_connector: &TlsConnector,
    write_half: &Arc<Mutex<Option<tokio::io::WriteHalf<TlsStream<TcpStream>>>>>,
    session_id: &Arc<Mutex<Option<u64>>>,
    config: &[PostProcessingConfig],
    msg2client_sender: &tokio::sync::broadcast::Sender<MessageToClient>,
    disconnect_tx: tokio::sync::mpsc::Sender<()>,
    is_resume: bool,
) -> bool {
    match connect_tls(tls_connector).await {
        Ok(stream) => {
            let (read_half, new_write_half) = tokio::io::split(stream);
            {
                let mut w = write_half.lock().await;
                *w = Some(new_write_half);
            }

            // Send StartSession or ResumeSession
            {
                let mut w = write_half.lock().await;
                if let Some(ref mut writer) = *w {
                    let msg = if is_resume {
                        let sid = session_id.lock().await;
                        MessageToPostProcessor::ResumeSession(sid.unwrap())
                    } else {
                        MessageToPostProcessor::StartSession
                    };
                    if let Err(e) = send_framed(writer, &msg).await {
                        eprintln!("Failed to send session init message: {:?}", e);
                        return false;
                    }
                    // Send current config
                    if let Err(e) = send_framed(writer, &MessageToPostProcessor::SetPostProcessorConfig(config.to_vec())).await {
                        eprintln!("Failed to send initial config to post-processing server: {:?}", e);
                    }
                }
            }

            // Spawn reader task
            let sender_clone = msg2client_sender.clone();
            let session_id_clone = session_id.clone();
            let write_half_clone = write_half.clone();
            tokio::spawn(async move {
                response_reader(read_half, sender_clone, session_id_clone, write_half_clone, disconnect_tx).await;
            });
            true
        }
        Err(e) => {
            eprintln!("Failed to connect to post-processing server: {:?}", e);
            false
        }
    }
}

/// Reads responses from the server at any time and forwards images to websocket clients.
async fn response_reader(
    mut read_half: tokio::io::ReadHalf<TlsStream<TcpStream>>,
    msg2client_sender: tokio::sync::broadcast::Sender<MessageToClient>,
    session_id: Arc<Mutex<Option<u64>>>,
    write_half: Arc<Mutex<Option<tokio::io::WriteHalf<TlsStream<TcpStream>>>>>,
    disconnect_tx: tokio::sync::mpsc::Sender<()>,
) {
    loop {
        match recv_framed(&mut read_half).await {
            Ok(bytes) => {
                enum Parsed { SessionId(u64), ReturnedImage(Vec<u8>), SaveToSystemStorage { filename: String, data: Vec<u8> }, Error }
                let parsed = match rkyv::from_bytes::<MessageToCameraServer>(&bytes) {
                    Ok(MessageToCameraServer::SessionId(id)) => Parsed::SessionId(id),
                    Ok(MessageToCameraServer::ReturnedImage(data)) => Parsed::ReturnedImage(data),
                    Ok(MessageToCameraServer::SaveToSystemStorage { filename, data }) => Parsed::SaveToSystemStorage { filename, data },
                    Err(e) => { eprintln!("Failed to deserialize server response: {:?}", e); Parsed::Error }
                };
                match parsed {
                    Parsed::SessionId(id) => {
                        println!("Session ID assigned by server: {}", id);
                        *session_id.lock().await = Some(id);
                    }
                    Parsed::ReturnedImage(image_data) => {
                        println!("Received processed image ({} bytes) from server.", image_data.len());
                        let image = Image::new(image_data, "image/jpeg".to_string(), ImageSource::PostProcessing);
                        let _ = msg2client_sender.send(MessageToClient::Image(image));
                    }
                    Parsed::SaveToSystemStorage { filename, data } => {
                        let dir = std::path::PathBuf::from("data");
                        if let Err(e) = std::fs::create_dir_all(&dir) {
                            eprintln!("Failed to create data directory: {:?}", e);
                        } else {
                            let path = dir.join(&filename);
                            match std::fs::write(&path, &data) {
                                Ok(_) => println!("Saved image to system storage: {:?}", path),
                                Err(e) => eprintln!("Failed to save image to system storage {:?}: {:?}", path, e),
                            }
                        }
                    }
                    Parsed::Error => {}
                }
            }
            Err(e) => {
                eprintln!("Server connection closed or read error: {:?}", e);
                // Clear write half so sends fail fast
                *write_half.lock().await = None;
                let _ = disconnect_tx.send(()).await;
                return;
            }
        }
    }
}

fn build_tls_connector() -> Result<TlsConnector, Box<dyn std::error::Error + Send + Sync>> {
    let cert_file = std::fs::File::open(CERT_PATH)?;
    let certs: Vec<rustls::pki_types::CertificateDer> =
        rustls_pemfile::certs(&mut BufReader::new(cert_file))
            .map(|r| r.expect("Invalid cert PEM"))
            .collect();

    let key_file = std::fs::File::open(KEY_PATH)?;
    let key = rustls_pemfile::private_key(&mut BufReader::new(key_file))?
        .ok_or("No private key found")?;

    let ca_file = std::fs::File::open(CA_PATH)?;
    let ca_certs: Vec<rustls::pki_types::CertificateDer> =
        rustls_pemfile::certs(&mut BufReader::new(ca_file))
            .map(|r| r.expect("Invalid CA cert PEM"))
            .collect();

    let mut root_store = RootCertStore::empty();
    for ca_cert in ca_certs {
        root_store.add(ca_cert)?;
    }

    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_client_auth_cert(certs, key)?;

    Ok(TlsConnector::from(Arc::new(config)))
}

async fn connect_tls(connector: &TlsConnector) -> Result<TlsStream<TcpStream>, Box<dyn std::error::Error + Send + Sync>> {
    let tcp = tokio::net::TcpStream::connect(format!("{}:{}", SERVER_HOST, SERVER_PORT)).await?;
    let server_name = ServerName::try_from(SERVER_HOST.to_string())?;
    let tls_stream = connector.connect(server_name, tcp).await?;
    Ok(tls_stream)
}

async fn send_framed<S, M>(stream: &mut S, message: &M) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWriteExt + Unpin,
    M: rkyv::Serialize<rkyv::ser::serializers::AllocSerializer<4096>>,
{
    let bytes = rkyv::to_bytes::<_, 4096>(message)
        .map_err(|e| format!("Serialization error: {:?}", e))?;
    let len = bytes.len() as u32;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(&bytes).await?;
    Ok(())
}

async fn recv_framed<S>(stream: &mut S) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncReadExt + Unpin,
{
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let msg_len = u32::from_be_bytes(len_buf) as usize;
    let mut buf = vec![0u8; msg_len];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn send_raw_to_server(
    writer: &mut tokio::io::WriteHalf<TlsStream<TcpStream>>,
    connection: &CameraConnection,
    image_path: &CameraFilePath,
) -> Result<(), CameraError> {
    let img = connection.camera.fs().download(&image_path.folder, &image_path.file).await?;
    let img_data = img.get_data(&connection.context).await?;

    println!("Sending raw image ({} bytes) to post-processing server...", img_data.len());

    send_framed(writer, &MessageToPostProcessor::RawImage(img_data.to_vec())).await
        .map_err(|e| CameraError::RsRawUtilError(format!("Send failed: {e}")))?;

    Ok(())
}
