use crate::websocket::scheme::{CameraStatus, MessageToClient, MessageToServer};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

#[derive(Clone)]
pub struct AppState {
    pub app2websocket: tokio::sync::broadcast::Sender<MessageToClient>,
    pub websocket2app: tokio::sync::mpsc::Sender<MessageToServer>,
    pub event_loop_paused: Arc<AtomicBool>,
    pub last_camera_status: Arc<Mutex<CameraStatus>>,
}
