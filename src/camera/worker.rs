use crate::camera::post_processing::start_post_processing_worker;
use crate::camera::camera_config::{get_camera_config, update_camera_config};
use crate::camera::capture::{capture_image, capture_preview_image};
use crate::camera::connection;
use crate::camera::connection::{connect_to_camera, get_camera_status};
use crate::storage::AppState;
use crate::websocket::scheme::{
    CameraDescriptor, CameraList, CameraStatus, ConnectToCamera, MessageToClient, MessageToServer,
};
use gphoto2::camera::CameraEvent;
use gphoto2::{Camera, Context};
use serde::{Deserialize, Serialize};
use std::mem::discriminant;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast::Sender;

#[derive(Clone)]
pub struct CameraConnection {
    pub camera: Camera,
    pub context: Context,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CameraError {
    Gphoto2Error(String),
    IoError(String),
    NotConnected,
    TokioError(String),
    InvalidConfigValueType,
    CaptureTookLongerThanInterval(u128),
    RsRawUtilError(String),
}

impl From<tokio::task::JoinError> for CameraError {
    fn from(e: tokio::task::JoinError) -> Self {
        CameraError::TokioError(e.to_string())
    }   
}

impl From<rsraw_utils::RsRawUtilsError> for CameraError {
    fn from(e: rsraw_utils::RsRawUtilsError) -> Self {
        CameraError::RsRawUtilError(e.to_string())
    }
}

impl From<gphoto2::Error> for CameraError {
    fn from(e: gphoto2::Error) -> Self {
        CameraError::Gphoto2Error(e.to_string())
    }
}

impl From<std::io::Error> for CameraError {
    fn from(e: std::io::Error) -> Self {
        CameraError::IoError(e.to_string())
    }
}

pub fn start_camera_event_loop(
    receiver: tokio::sync::broadcast::Receiver<CameraConnection>,
) -> tokio::sync::broadcast::Sender<Arc<CameraEvent>> {
    let mut receiver = receiver;

    let (sender_orig, _) = tokio::sync::broadcast::channel::<Arc<CameraEvent>>(50);

    let sender = sender_orig.clone();
    tokio::spawn(async move {
        let mut connection = match receiver.recv().await {
            Ok(conn) => conn,
            Err(e) => {
                println!("Channel to camera event loop closed {e}.");
                return;
            }
        };

        loop {
            tokio::select! {
                new_conn = receiver.recv() => {
                    if let Ok(new_conn) = new_conn{
                        connection = new_conn;
                    }else{
                        println!("Channel to camera event loop closed.");
                        break
                    }
                },
                event = connection.camera.wait_event(Duration::from_millis(100)) => {
                    match event{
                        Ok(event) => {
                            match event{
                                CameraEvent::Timeout => {},
                                _ => {
                                    let _ = sender.send(Arc::new(event));
                                }
                            }
                        },
                        Err(e) => {
                            println!("Error receiving camera event: {:?}", e);
                            tokio::time::sleep(Duration::from_millis(1000)).await;
                        }
                    }
                }
            }
        }
    });

    sender_orig
}

async fn wait_for_event_inner(
    mut receiver: tokio::sync::broadcast::Receiver<Arc<CameraEvent>>,
    event_type: CameraEvent,
) -> Option<Arc<CameraEvent>> {
    loop {
        let msg = receiver.recv().await;
        if let Ok(msg) = msg {
            if discriminant(&(*msg)) == discriminant(&event_type) {
                return Some(msg);
            }
        }
    }
}

pub async fn wait_for_event(
    receiver: tokio::sync::broadcast::Receiver<Arc<CameraEvent>>,
    timeout: Duration,
    event_type: CameraEvent,
) -> Option<Arc<CameraEvent>> {
    tokio::select! {
        msg = wait_for_event_inner(receiver, event_type) => {
            return msg;
        },
        _ = tokio::time::sleep(timeout) => {
            return None;
        },
    }
}

pub fn start_worker(state: AppState, mut receiver: tokio::sync::mpsc::Receiver<MessageToServer>) {
    let sender = state.app2websocket.clone();

    let mut connection: Option<CameraConnection> = None;
    let mut last_port: Option<String> = None;
    let mut last_model: Option<String> = None;

    let (sender_to_workers, recv) = tokio::sync::broadcast::channel::<CameraConnection>(1);
    let event_channel = start_camera_event_loop(recv);

    let (cancel_channel_sender, _) = tokio::sync::broadcast::channel::<()>(1);

    let pp_worker_sender = start_post_processing_worker(
        event_channel.clone(),
        sender_to_workers.subscribe(),
        sender.clone(),
    );

    tokio::spawn(async move {
        loop {
            match &mut connection {
                // no connection established yet, reduced mode
                None => {
                    let msg = match receiver.recv().await {
                        Some(msg) => msg,
                        None => {
                            eprintln!("websocket2app channel closed.");
                            break;
                        }
                    };
                    println!("Received websocket2app message: {:?}", msg);
                    match msg {
                        MessageToServer::GetDetectedCameras => {
                            let _ = sender.send(list_available_cameras().await.into());
                        }
                        MessageToServer::GetCameraStatus => {
                            let _ =
                                sender.send(MessageToClient::CameraStatus(CameraStatus::default()));
                        }
                        MessageToServer::ConnectToCamera(cfg) => {
                            last_port = cfg.port.clone();
                            last_model = cfg.model.clone();
                            let connect_result = if let (Some(port), Some(model)) = (&last_port, &last_model) {
                                connect_to_camera(port, model).await
                            } else {
                                connection::autoconnect_camera().await
                            };
                            match connect_result {
                                Ok(conn) => {
                                    let _ = sender_to_workers.send(conn.clone());
                                    connection = Some(conn);

                                    tokio::time::sleep(Duration::from_millis(500)).await;

                                    if let Some(conn) = connection.as_mut() {
                                        let mut retries = 0u64;
                                        loop {
                                            match get_camera_status(conn).await {
                                                Ok(status) => {
                                                    let _ = sender.send(MessageToClient::CameraStatus(status));
                                                    break;
                                                }
                                                Err(CameraError::Gphoto2Error(ref msg)) if msg.contains("io in progress") && retries < 5 => {
                                                    retries += 1;
                                                    tokio::time::sleep(Duration::from_millis(300 * retries)).await;
                                                }
                                                Err(e) => {
                                                    let _ = sender.send(MessageToClient::CameraError(e));
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = sender.send(MessageToClient::CameraError(e));
                                }
                            }
                        }
                        MessageToServer::SetPostProcessingConfigs(configs) => {
                            let _ = pp_worker_sender.send(configs).await;
                        }
                        MessageToServer::Shutdown => {
                            println!("Shutting down system...");
                            let _ = std::process::Command::new("sudo")
                                .args(&["shutdown", "-h", "now"])
                                .spawn();
                        }
                        _ => {
                            // Need to establish connection first for all other messages
                            if last_port.is_none() || last_model.is_none() {
                                let _ = sender
                                    .send(MessageToClient::CameraError(CameraError::NotConnected));
                            } else {
                                match connection::connect(ConnectToCamera {
                                    port: last_port.clone(),
                                    model: last_model.clone(),
                                })
                                .await
                                {
                                    Ok(new_con) => {
                                        let _ = sender_to_workers.send(new_con.clone());
                                        connection = Some(new_con);
                                    }
                                    Err(e) => {
                                        let _ = sender.send(MessageToClient::CameraError(e));
                                    }
                                }
                            }
                        }
                    }
                }
                // connection established, full mode
                Some(conn) => {
                    let msg = match receiver.recv().await {
                        Some(msg) => msg,
                        None => {
                            eprintln!("websocket2app channel closed.");
                            break;
                        }
                    };
                    let mut res = Ok(());
                    let mut retries = 0;
                    while retries < 3 {
                        match handle_msg(
                            &msg,
                            conn,
                            sender.clone(),
                            sender_to_workers.clone(),
                            cancel_channel_sender.clone(),
                            event_channel.clone(),
                            pp_worker_sender.clone(),
                        )
                        .await
                        {
                            Ok(_) => {
                                res = Ok(());
                                break;
                            }
                            Err(e) => {
                                println!(
                                    "Got camera error {:?}. Trying to reconnect to camera and trying again.",
                                    e
                                );
                                res = Err(e);
                                retries += 1;

                                if let Ok(new_con) =
                                    connection::connect(ConnectToCamera::default()).await
                                {
                                    let _ = sender_to_workers.send(new_con.clone());
                                    *conn = new_con;
                                }
                            }
                        }
                    }
                    if let Err(e) = res {
                        let _ = sender.send(MessageToClient::CameraError(e));
                    }
                }
            }
        }
    });
}

async fn list_available_cameras() -> Result<CameraList, CameraError> {
    let context = Context::new()?;
    let list = context.list_cameras().await?;

    let mut res = vec![];
    for camera in list {
        res.push(CameraDescriptor {
            model: camera.model,
            port: camera.port,
        });
    }
    Ok(res.into())
}

async fn handle_msg(
    msg: &MessageToServer,
    connection: &mut CameraConnection,
    sender: Sender<MessageToClient>,
    sender_to_event_loop: tokio::sync::broadcast::Sender<CameraConnection>,
    cancel_channel_sender: tokio::sync::broadcast::Sender<()>,
    event_channel: tokio::sync::broadcast::Sender<Arc<CameraEvent>>,
    pp_worker_sender: tokio::sync::mpsc::Sender<Vec<SmartCameraTethering2_shared_types::PostProcessingConfig>>,
) -> Result<(), CameraError> {
    println!("Handling websocket2app message: {:?}", msg);
    match msg {
        MessageToServer::GetDetectedCameras => {
            let _ = sender.send(Ok(list_available_cameras().await?).into());
        }
        MessageToServer::GetCameraStatus => {
            let _ = sender.send(Ok(connection::get_camera_status(connection).await?).into());
        }
        MessageToServer::ConnectToCamera(settings) => {
            let new_connection = connection::connect(settings.clone()).await?;
            let _ = sender_to_event_loop.send(new_connection.clone());
            *connection = new_connection;

            tokio::time::sleep(Duration::from_millis(500)).await;

            let mut retries = 0u64;
            loop {
                match get_camera_status(connection).await {
                    Ok(status) => {
                        let _ = sender.send(MessageToClient::CameraStatus(status));
                        break;
                    }
                    Err(CameraError::Gphoto2Error(ref msg)) if msg.contains("io in progress") && retries < 5 => {
                        retries += 1;
                        tokio::time::sleep(Duration::from_millis(300 * retries)).await;
                    }
                    Err(e) => {
                        let _ = sender.send(MessageToClient::CameraError(e));
                        break;
                    }
                }
            }
        }
        MessageToServer::GetCameraConfig => {
            let _ = sender.send(Ok(get_camera_config(connection).await?).into());
        }
        MessageToServer::CapturePreviewImage => {
            let img = capture_preview_image(connection).await?;
            let _ = sender.send(MessageToClient::Image(img));
        }
        MessageToServer::UpdateCameraConfig(update) => {
            update_camera_config(connection, update.clone()).await?;
        }
        MessageToServer::CaptureImage(capture_settings) => {
            capture_image(
                connection,
                capture_settings.clone(),
                cancel_channel_sender.subscribe(),
                event_channel,
                sender.clone(),
            )
            .await?;
        }
        MessageToServer::CancelCapture => {
            if let Err(e) = cancel_channel_sender.send(()) {
                match e {
                    tokio::sync::broadcast::error::SendError(_) => {
                        // No one is listening, that's fine (no capture in progress)
                        println!("No active capture to cancel.");
                    }
                }
            }
        }
        MessageToServer::SetPostProcessingConfigs(configs) => {
            let _ = pp_worker_sender.send(configs.clone()).await;
        }
        MessageToServer::Shutdown => {
            println!("Shutting down system...");
            let _ = std::process::Command::new("sudo")
                .args(&["shutdown", "-h", "now"])
                .spawn();
        }
    }
    Ok(())
}
