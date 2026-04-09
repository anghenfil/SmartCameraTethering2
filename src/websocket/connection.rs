use crate::storage::AppState;
use crate::websocket::scheme::{MessageToClient, MessageToServer, WebSocketMsgType};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::Response;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};

pub(crate) async fn handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (sender, receiver) = socket.split();

    tokio::spawn(socket_writer(sender, state.clone()));
    tokio::spawn(socket_reader(receiver, state.clone()));
}

async fn socket_writer(mut sender: SplitSink<WebSocket, Message>, state: AppState) {
    let mut receiver = state.app2websocket.subscribe();

    while let Ok(msg) = receiver.recv().await {
        if let Err(e) = msg.send(&mut sender).await {
            if let WebsocketError::AxumError(e) = e {
                println!("Websocket closed: {}", e);
                break;
            } else {
                eprintln!("Error sending message via websocket to client: {:?}", e);
            }
        }
    }
}

#[derive(Debug)]
pub enum WebsocketError {
    SerializationError(serde_json::Error),
    AxumError(axum::Error),
}

impl From<serde_json::Error> for WebsocketError {
    fn from(err: serde_json::Error) -> Self {
        WebsocketError::SerializationError(err)
    }
}

impl From<axum::Error> for WebsocketError {
    fn from(err: axum::Error) -> Self {
        WebsocketError::AxumError(err)
    }
}

impl MessageToClient {
    pub async fn send(
        self,
        sender: &mut SplitSink<WebSocket, Message>,
    ) -> Result<(), WebsocketError> {
        match self {
            MessageToClient::Image(img) => {
                // Send image metadata as json
                let (image_info, mut data) = img.split();
                let mut json = serde_json::to_vec(&image_info)?;

                json.insert(0, WebSocketMsgType::JSON.into());

                sender.send(Message::Binary(json.into())).await?;

                // Send image data as binary
                data.insert(0, WebSocketMsgType::BinaryImage.into());
                sender.send(Message::Binary(data.into())).await?;
                Ok(())
            }
            MessageToClient::ImageList(mut imgs) => {
                while let Some(img) = imgs.pop() {
                    // Send image metadata as json
                    let (image_info, mut data) = img.split();
                    let mut json = serde_json::to_vec(&image_info)?;

                    json.insert(0, WebSocketMsgType::JSON.into());

                    sender.send(Message::Binary(json.into())).await?;

                    // Send image data as binary
                    data.insert(0, WebSocketMsgType::BinaryImage.into());
                    sender.send(Message::Binary(data.into())).await?;
                }
                Ok(())
            }
            msg => {
                let mut json = serde_json::to_vec(&msg)?;

                // Prepend Message Type
                json.insert(0, WebSocketMsgType::JSON.into());
                sender.send(Message::Binary(json.into())).await?;
                Ok(())
            }
        }
    }
}

async fn socket_reader(mut receiver: SplitStream<WebSocket>, state: AppState) {
    let sender = state.websocket2app.clone();

    while let Some(msg) = receiver.next().await {
        if let Ok(msg) = msg {
            if let Message::Binary(data) = msg {
                let msg = match serde_json::from_slice::<MessageToServer>(&data) {
                    Ok(msg) => msg,
                    Err(e) => {
                        eprintln!("Error parsing websocket message: {:?}", e);
                        continue;
                    }
                };
                if let Err(e) = sender.send(msg).await {
                    eprintln!("Failed to send message to app: {:?}", e);
                }
            }
        }
    }
}
