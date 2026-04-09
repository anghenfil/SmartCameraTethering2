use crate::camera::worker::start_worker;
use crate::storage::AppState;
use axum::Router;
use axum::routing::{any, get};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tower_http::services::ServeDir;

pub mod camera;
pub mod storage;
pub mod webgui;
pub mod websocket;

#[tokio::main]
async fn main() {
    rustls::crypto::ring::default_provider().install_default()
        .expect("Failed to install rustls crypto provider");

    let (websocket2app_tx, websocket2app_rx) = tokio::sync::mpsc::channel::<crate::websocket::scheme::MessageToServer>(32);

    let storage = AppState {
        app2websocket: tokio::sync::broadcast::Sender::new(10),
        websocket2app: websocket2app_tx,
        event_loop_paused: Arc::new(AtomicBool::new(false)),
    };

    let storage_cpy = storage.clone();
    start_worker(storage_cpy.clone(), websocket2app_rx);

    let app = Router::new()
        .route("/ws", any(websocket::connection::handler))
        .route("/", get(webgui::index))
        .nest_service("/assets", ServeDir::new("assets"))
        .with_state(storage);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
