use crate::websocket::scheme::{MessageToClient, MessageToServer};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

#[derive(Clone)]
pub struct AppState {
    pub app2websocket: tokio::sync::broadcast::Sender<MessageToClient>,
    pub websocket2app: tokio::sync::mpsc::Sender<MessageToServer>,
    pub event_loop_paused: Arc<AtomicBool>,
}
