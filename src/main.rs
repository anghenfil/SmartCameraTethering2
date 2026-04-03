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
    let storage = AppState {
        app2websocket: tokio::sync::broadcast::Sender::new(10),
        websocket2app: tokio::sync::broadcast::Sender::new(10),
        event_loop_paused: Arc::new(AtomicBool::new(false)),
    };

    let storage_cpy = storage.clone();
    start_worker(storage_cpy.clone());

    let app = Router::new()
        .route("/ws", any(websocket::connection::handler))
        .route("/", get(webgui::index))
        .nest_service("/assets", ServeDir::new("assets"))
        .with_state(storage);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
