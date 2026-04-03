use crate::camera::worker::{CameraConnection, CameraError, wait_for_event};
use crate::websocket::scheme::{CaptureImage, Image, ImageSource, MessageToClient};
use gphoto2::camera::CameraEvent;
use std::sync::Arc;
use std::time::Duration;

pub async fn capture_preview_image(
    connection: &mut CameraConnection,
) -> Result<Image, CameraError> {
    let preview_image = connection.camera.capture_preview().await?;
    let imgdata = preview_image.get_data(&connection.context).await?;
    let mime_type = preview_image.mime_type();

    Ok(Image::new(imgdata.into(), mime_type, ImageSource::Preview))
}

pub async fn capture_image(
    connection: &mut CameraConnection,
    capture_settings: CaptureImage,
    cancel_channel_receiver: tokio::sync::broadcast::Receiver<()>,
    event_channel: tokio::sync::broadcast::Sender<Arc<CameraEvent>>,
    msg2client_sender: tokio::sync::broadcast::Sender<MessageToClient>,
) -> Result<(), CameraError> {
    capture_image_work(
        connection,
        capture_settings,
        cancel_channel_receiver,
        event_channel,
        msg2client_sender,
    )
    .await?;
    Ok(())
}

async fn capture_image_work(
    connection: &mut CameraConnection,
    capture_settings: CaptureImage,
    mut cancel_channel_receiver: tokio::sync::broadcast::Receiver<()>,
    event_channel: tokio::sync::broadcast::Sender<Arc<CameraEvent>>,
    msg2client_sender: tokio::sync::broadcast::Sender<MessageToClient>,
) -> Result<(), CameraError> {
    let camera = connection.camera.clone();
    tokio::task::spawn(async move {
        match capture_settings {
            CaptureImage::Single => {
                if let Err(e) = camera.trigger_capture().await {
                    eprintln!("Error triggering capture: {:?}", e);
                    let _ = msg2client_sender.send(MessageToClient::CameraError(e.into()));
                } else {
                    let _ = msg2client_sender.send(MessageToClient::CaptureComplete);
                }
            }
            CaptureImage::Interval(interval_settings) => {
                if let Some(number_of_images) = interval_settings.number_of_images {
                    for i in 0..number_of_images {
                        tokio::select! {
                            _ = cancel_channel_receiver.recv() => {
                                println!("Canceling interval capture at image {i}");
                                let _ = msg2client_sender.send(MessageToClient::CaptureCancelled);
                                return;
                            },
                            _ = async {
                                println!("Capturing image {i} of {number_of_images}");

                                let time = std::time::SystemTime::now();

                                if let Err(e) = camera.trigger_capture().await{
                                    eprintln!("Error capturing image: {:?}", e);
                                    let _ = msg2client_sender.send(MessageToClient::CameraError(e.into()));
                                }
                                if let None = wait_for_event(event_channel.subscribe(), Duration::from_secs(5), CameraEvent::CaptureComplete).await{
                                    eprintln!("Timeout waiting for capture complete event");
                                }
                                let time_taken = time.elapsed().unwrap();
                                println!("Image capture took {} ms", time_taken.as_millis());
                                let sleep_time = interval_settings.interval_ms.saturating_sub(time_taken.as_millis() as u32);
                                if sleep_time > 0{
                                    println!("Sleeping {} ms", sleep_time);
                                    tokio::time::sleep(Duration::from_millis(sleep_time as u64)).await;
                                }else{
                                    let _ = msg2client_sender.send(MessageToClient::CameraError(CameraError::CaptureTookLongerThanInterval(time_taken.as_millis())));
                                }
                            } => {}
                        }
                    }
                    let _ = msg2client_sender.send(MessageToClient::CaptureComplete);
                } else {
                    unimplemented!("Missing number of images for interval capture");
                }
            }
            _ => {
                unimplemented!("Unsupported capture mode")
            }
        }
    });
    Ok(())
}
