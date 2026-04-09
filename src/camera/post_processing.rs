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
use SmartCameraTethering2_shared_types::{MessageToCameraServer, MessageToPostProcessor};
use crate::websocket::scheme::PostProcessingConfig;
use crate::camera::worker::{CameraConnection, CameraError};
use crate::websocket::scheme::{Image, ImageSource, MessageToClient};

const SERVER_HOST: &str = "anghenfil.de";
const SERVER_PORT: u16 = 9000;
const CERT_PATH: &str = "certs/client.crt";
const KEY_PATH: &str = "certs/client.key";
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

/// Item queued for post-processing: a RAW file to be sent to the server.
struct PostProcessingItem {
    conn: CameraConnection,
    file_path: CameraFilePath,
    compress: bool,
    compression_level: u8,
    enqueued_at: std::time::Instant,
}

const POST_PROCESSING_TIMEOUT: tokio::time::Duration = tokio::time::Duration::from_secs(30);

pub fn start_post_processing_worker(
    event_channel: tokio::sync::broadcast::Sender<Arc<CameraEvent>>,
    mut connection_channel: tokio::sync::broadcast::Receiver<Option<CameraConnection>>,
    msg2client_sender: tokio::sync::broadcast::Sender<MessageToClient>,
) -> tokio::sync::mpsc::Sender<Vec<PostProcessingConfig>> {
    let (sender, mut recv) = tokio::sync::mpsc::channel::<Vec<PostProcessingConfig>>(3);

    // Queue for RAW images waiting to be sent to the post-processing server (one at a time)
    let (queue_tx, queue_rx) = tokio::sync::mpsc::channel::<PostProcessingItem>(64);

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

        // Spawn the single post-processing queue worker
        {
            let write_half = write_half.clone();
            let session_id = session_id.clone();
            let msg2client_sender = msg2client_sender.clone();
            let disconnect_tx = disconnect_tx.clone();
            let tls_connector = tls_connector.clone();
            tokio::task::spawn(post_processing_queue_worker(
                queue_rx,
                write_half,
                session_id,
                msg2client_sender,
                disconnect_tx,
                tls_connector,
            ));
        }

        let mut event_channel_receiver = event_channel.subscribe();
        loop {
            tokio::select! {
                new_config = recv.recv() => {
                    match new_config {
                        Some(new_config) => {
                            println!("Received new post processing config: {:?}", new_config);
                            config = new_config;
                            let mut w = write_half.lock().await;
                            if w.is_none() {
                                drop(w);
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
                            } else {
                                if let Some(ref mut writer) = *w {
                                    if let Err(e) = send_framed(writer, &MessageToPostProcessor::SetPostProcessorConfig(config.iter().cloned().map(Into::into).collect())).await {
                                        eprintln!("Failed to send config to post-processing server: {:?}", e);
                                    }
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
                                let event_received_at = std::time::Instant::now();
                                let file_path: CameraFilePath = new_file.into();
                                println!("[TIMING] NewFile event received: {}/{}", file_path.folder, file_path.file);
                                let name_lower = file_path.file.to_lowercase();

                                if name_lower.ends_with(".jpg") || name_lower.ends_with(".jpeg") {
                                    // JPG: download and send directly to client
                                    if let Some(conn) = &mut connection {
                                        println!("[TIMING] +{:.1}ms — starting JPG download", event_received_at.elapsed().as_secs_f64() * 1000.0);
                                        match conn.camera.fs().download(&file_path.folder, &file_path.file).await {
                                            Ok(img) => match img.get_data(&conn.context).await {
                                                Ok(data) => {
                                                    println!("[TIMING] +{:.1}ms — JPG downloaded ({} bytes), sending to client", event_received_at.elapsed().as_secs_f64() * 1000.0, data.len());
                                                    let image = Image::new(data.to_vec(), "image/jpeg".to_string(), ImageSource::Capture);
                                                    let _ = msg2client_sender.send(MessageToClient::Image(image));
                                                    println!("[TIMING] +{:.1}ms — JPG sent to client", event_received_at.elapsed().as_secs_f64() * 1000.0);
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
                                    // RAW: enqueue for sequential post-processing (one at a time)
                                    println!("[TIMING] +{:.1}ms — RAW file, enqueueing for post-processing", event_received_at.elapsed().as_secs_f64() * 1000.0);
                                    if let Some(conn) = &connection {
                                        let compress = config.iter().any(|c| c.compress);
                                        let compression_level = config.iter().filter(|c| c.compress).map(|c| c.compression_level).max().unwrap_or(1);
                                        let item = PostProcessingItem {
                                            conn: conn.clone(),
                                            file_path,
                                            compress,
                                            compression_level,
                                            enqueued_at: event_received_at,
                                        };
                                        if let Err(e) = queue_tx.send(item).await {
                                            eprintln!("Post-processing queue full or closed: {:?}", e);
                                        }
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
                        Ok(Some(new_connection)) => {
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
                        Ok(None) => {}
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

/// Single task that drains the post-processing queue one image at a time,
/// applying a ~30-second timeout per image upload.
async fn post_processing_queue_worker(
    mut queue_rx: tokio::sync::mpsc::Receiver<PostProcessingItem>,
    write_half: Arc<Mutex<Option<tokio::io::WriteHalf<TlsStream<TcpStream>>>>>,
    session_id: Arc<Mutex<Option<u64>>>,
    msg2client_sender: tokio::sync::broadcast::Sender<MessageToClient>,
    disconnect_tx: tokio::sync::mpsc::Sender<()>,
    tls_connector: TlsConnector,
) {
    while let Some(item) = queue_rx.recv().await {
        println!(
            "[TIMING] +{:.1}ms — post-processing queue worker: processing item {}",
            item.enqueued_at.elapsed().as_secs_f64() * 1000.0,
            item.file_path.file
        );

        let mut last_err = None;
        for attempt in 1..=3 {
            println!(
                "[TIMING] +{:.1}ms — attempt {}/3: acquiring write lock",
                item.enqueued_at.elapsed().as_secs_f64() * 1000.0,
                attempt
            );
            let mut w = write_half.lock().await;
            println!(
                "[TIMING] +{:.1}ms — attempt {}/3: lock acquired",
                item.enqueued_at.elapsed().as_secs_f64() * 1000.0,
                attempt
            );

            if let Some(ref mut writer) = *w {
                println!(
                    "[TIMING] +{:.1}ms — attempt {}/3: starting send_raw_to_server",
                    item.enqueued_at.elapsed().as_secs_f64() * 1000.0,
                    attempt
                );
                let send_fut = send_raw_to_server(writer, &item.conn, &item.file_path, item.compress, item.compression_level);
                match tokio::time::timeout(POST_PROCESSING_TIMEOUT, send_fut).await {
                    Ok(Ok(_)) => {
                        println!(
                            "[TIMING] +{:.1}ms — attempt {}/3: send_raw_to_server succeeded",
                            item.enqueued_at.elapsed().as_secs_f64() * 1000.0,
                            attempt
                        );
                        last_err = None;
                        break;
                    }
                    Ok(Err(e)) => {
                        eprintln!("Error sending raw to post-processing server (attempt {}/3): {:?}", attempt, e);
                        last_err = Some(e);
                        *w = None;
                        drop(w);
                    }
                    Err(_elapsed) => {
                        eprintln!("Timeout sending raw to post-processing server (attempt {}/3)", attempt);
                        last_err = Some(CameraError::RsRawUtilError("Upload timed out after 30s".to_string()));
                        *w = None;
                        drop(w);
                    }
                }
            } else {
                // No connection — try to establish one before retrying
                drop(w);
                let sid = *session_id.lock().await;
                let is_resume = sid.is_some();
                establish_connection(
                    &tls_connector,
                    &write_half,
                    &session_id,
                    &[],
                    &msg2client_sender,
                    disconnect_tx.clone(),
                    is_resume,
                ).await;
            }

            if attempt < 3 {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                // Re-establish connection after a send failure
                if write_half.lock().await.is_none() {
                    let sid = *session_id.lock().await;
                    let is_resume = sid.is_some();
                    establish_connection(
                        &tls_connector,
                        &write_half,
                        &session_id,
                        &[],
                        &msg2client_sender,
                        disconnect_tx.clone(),
                        is_resume,
                    ).await;
                }
            }
        }

        if let Some(e) = last_err {
            let _ = msg2client_sender.send(MessageToClient::CameraError(e));
            let _ = disconnect_tx.send(()).await;
        }
    }
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
                    if let Err(e) = send_framed(writer, &MessageToPostProcessor::SetPostProcessorConfig(config.iter().cloned().map(Into::into).collect())).await {
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
    stream.flush().await?;
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
    compress: bool,
    compression_level: u8,
) -> Result<(), CameraError> {
    let t0 = std::time::Instant::now();
    let img = connection.camera.fs().download(&image_path.folder, &image_path.file).await?;
    println!("[TIMING] send_raw_to_server: fs().download() done in {:.1}ms", t0.elapsed().as_secs_f64() * 1000.0);
    let img_data = img.get_data(&connection.context).await?;
    println!("[TIMING] send_raw_to_server: get_data() done in {:.1}ms ({} bytes total)", t0.elapsed().as_secs_f64() * 1000.0, img_data.len());

    let msg = if compress {
        let t_compress = std::time::Instant::now();
        // zstd level 1: fastest, good ratio, suitable for Raspberry Pi Zero 2
        let compressed = zstd::encode_all(img_data.as_ref(), compression_level as i32)
            .map_err(|e| CameraError::RsRawUtilError(format!("Compression failed: {e}")))?;
        println!("[TIMING] send_raw_to_server: zstd compression done in {:.1}ms ({} -> {} bytes, {:.1}% of original)",
            t_compress.elapsed().as_secs_f64() * 1000.0, img_data.len(), compressed.len(),
            compressed.len() as f64 / img_data.len() as f64 * 100.0);
        MessageToPostProcessor::CompressedRawImage(compressed)
    } else {
        MessageToPostProcessor::RawImage(img_data.to_vec())
    };

    send_framed(writer, &msg).await
        .map_err(|e| CameraError::RsRawUtilError(format!("Send failed: {e}")))?;
    println!("[TIMING] send_raw_to_server: send_framed() done in {:.1}ms", t0.elapsed().as_secs_f64() * 1000.0);

    Ok(())
}
