use std::path::PathBuf;
use crate::websocket::scheme::{Image, ImageSource, MessageToClient, OutputDestination, OutputFormat, PostProcessingConfig, PostProcessingStep, UploadDestination};
use gphoto2::camera::CameraEvent;
use std::sync::Arc;
use rsraw::RawImage;
use crate::camera::worker::{CameraConnection, CameraError};

#[derive(Debug, Clone)]
struct CameraFilePath{
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
        let mut config = vec![];
        let mut image_paths : Vec<CameraFilePath> = Vec::new();
        let mut connection : Option<CameraConnection> = None;


        let mut event_channel_receiver = event_channel.subscribe();
        loop {
            tokio::select! {
                new_config = recv.recv() => {
                    match new_config{
                        Some(new_config) => {
                            println!("Received new post processing config: {:?}", new_config);
                            config = new_config;
                        },
                        None => {
                            eprintln!("Channel to post processing worker closed.");
                            return;
                        }
                    }
                },
                event = event_channel_receiver.recv() => {
                    match event{
                        Ok(event) => {
                            // We only care for new file events
                            if let CameraEvent::NewFile(new_file) = event.as_ref(){
                                image_paths.push(new_file.into());
                            }

                            for pp_config in &config{
                                if pp_config.trigger_every_n_images == 0{
                                    continue;
                                }
                                if (image_paths.len() % pp_config.trigger_every_n_images as usize) == 0{
                                    if let Some(conn) = &mut connection{
                                        if let Err(e) = apply_post_processing(&pp_config, &image_paths, conn, &msg2client_sender).await {
                                            eprintln!("Error in post processing: {:?}", e);
                                            let _ = msg2client_sender.send(MessageToClient::CameraError(e));
                                        }
                                    }
                                }
                            }
                        },
                        Err(e) => {
                            eprintln!("Error receiving camera event: {:?}", e);
                            return;
                        }
                    }
                },
                new_connection = connection_channel.recv() => {
                    match new_connection{
                        Ok(new_connection) => {
                            connection = Some(new_connection);
                        },
                        Err(e) => {
                            eprintln!("Channel to post processing worker closed. {e}");
                        }
                    }
                }
            }
        }
    });

    sender
}

async fn apply_post_processing(pp_config: &PostProcessingConfig, image_paths: &Vec<CameraFilePath>, connection: &mut CameraConnection, msg2client_sender: &tokio::sync::broadcast::Sender<MessageToClient>) -> Result<(), CameraError>{
    if let Some(last_image_path) = image_paths.last(){
        let id = uuid::Uuid::new_v4();
        let temp_folder = PathBuf::from(format!("temp/{}", id));
        tokio::fs::create_dir_all(&temp_folder).await?;

        // 1. Load image from camera, currently we always expect a raw image
        let img = connection.camera.fs().download(&last_image_path.folder, &last_image_path.file).await?;
        let img_data = img.get_data(&connection.context).await?;
        let mut processed_raw = Some(tokio::task::spawn_blocking(move || -> Result<RawImage, CameraError> {
            let raw = RawImage::open(&img_data).map_err(|e| CameraError::RsRawUtilError(format!("Couldn't open image form camera as raw: {e}.")))?;
            Ok(raw)
        }).await??);

        let base_tiff_path = temp_folder.join("base.tiff");
        let mut base_tiff_exists = false;
        let base_tiff_path_cloned = base_tiff_path.clone();

        for step in &pp_config.steps{
            let ensure_base_tiff = |processed_raw: &mut Option<RawImage>, base_tiff_exists: &mut bool| -> Result<bool, CameraError> {
                if *base_tiff_exists {
                    return Ok(true);
                }
                if let Some(current_raw) = processed_raw.take() {
                    let dest = base_tiff_path_cloned.clone();
                    let runtime = tokio::runtime::Handle::current();
                    runtime.block_on(tokio::task::spawn_blocking(move || -> Result<(), CameraError> {
                        rsraw_utils::convert_raw(current_raw, rsraw_utils::OutputFormat::TIFF, &dest).map_err(|e| CameraError::RsRawUtilError(e.to_string()))?;
                        Ok(())
                    })).map_err(|e| CameraError::RsRawUtilError(format!("Blocking task failed: {}", e)))??;
                    *base_tiff_exists = true;
                    Ok(true)
                } else {
                    Ok(false)
                }
            };

            match step{
                PostProcessingStep::Blend(blend_settings) => {
                    if blend_settings.number_of_images < 2{
                        return Err(CameraError::RsRawUtilError("At least 2 images are required for blending.".to_string()));
                    }
                    if processed_raw.is_none() {
                        return Err(CameraError::RsRawUtilError("No image available for blending.".to_string()));
                    }

                    // 2. Open images as raw image
                    let connection = connection.clone();
                    let image_paths = image_paths.clone();
                    let blend_settings = blend_settings.clone();
                    let image_paths_len = image_paths.len();
                    let current_raw = processed_raw.take().unwrap();

                    processed_raw = Some(tokio::task::spawn_blocking(async move || -> Result<RawImage, CameraError> {
                        let mut images_to_blend: Vec<RawImage> = vec![current_raw];

                        let max_image_path_index = image_paths_len-1;

                        let start = if max_image_path_index >= (blend_settings.number_of_images as usize - 1) {
                            max_image_path_index - (blend_settings.number_of_images as usize - 1)
                        } else {
                            0
                        };

                        for i in start..max_image_path_index {
                            let img_path = &image_paths[i];
                            let img = connection.camera.fs().download(&img_path.folder, &img_path.file).await?;
                            let img_data = img.get_data(&connection.context).await?;
                            let img = RawImage::open(&img_data).map_err(|_| CameraError::RsRawUtilError("Couldn't open image form camera as raw.".to_string()))?;
                            images_to_blend.push(img);
                        }
                        rsraw_utils::blend_raw_images(images_to_blend, blend_settings.blending_mode).map_err(|e|e.into())
                    }).await?.await?);

                    // Reset base tiff exists so it gets regenerated from the blended image
                    base_tiff_exists = false;
                }
                PostProcessingStep::Convert(convert_settings) => {
                    if !ensure_base_tiff(&mut processed_raw, &mut base_tiff_exists)? {
                        return Err(CameraError::RsRawUtilError("No image available for conversion.".to_string()));
                    }

                    for format in &convert_settings.output_formats {
                        let extension = match format {
                            OutputFormat::JPEG => "jpg",
                            OutputFormat::PNG => "png",
                            OutputFormat::TIFF => "tiff",
                            OutputFormat::RAW => continue,
                        };

                        let dest_path = temp_folder.join(format!("image.{}", extension));

                        if matches!(format, OutputFormat::TIFF) {
                            tokio::fs::copy(&base_tiff_path_cloned, &dest_path).await?;
                            continue;
                        }

                        let src = base_tiff_path_cloned.clone();
                        let format_to_save = match format {
                             OutputFormat::JPEG => image::ImageFormat::Jpeg,
                             OutputFormat::PNG => image::ImageFormat::Png,
                             _ => unreachable!(),
                        };

                        tokio::task::spawn_blocking(move || -> Result<(), CameraError> {
                            let img = image::open(&src).map_err(|e| CameraError::RsRawUtilError(format!("Failed to open base tiff: {}", e)))?;
                            img.save_with_format(&dest_path, format_to_save).map_err(|e| CameraError::RsRawUtilError(format!("Failed to save converted image: {}", e)))?;
                            Ok(())
                        }).await??;
                    }
                }
                PostProcessingStep::Save(save_settings) => {
                    if !ensure_base_tiff(&mut processed_raw, &mut base_tiff_exists)? {
                        return Err(CameraError::RsRawUtilError("No image available for saving.".to_string()));
                    }

                    match save_settings.output_destination {
                        OutputDestination::SystemStorage => {
                            let data_folder = PathBuf::from("data").join(id.to_string());
                            tokio::fs::create_dir_all(&data_folder).await?;
                            let mut entries = tokio::fs::read_dir(&temp_folder).await?;
                            while let Some(entry) = entries.next_entry().await? {
                                let dest = data_folder.join(entry.file_name());
                                tokio::fs::copy(entry.path(), dest).await?;
                            }
                        }
                        OutputDestination::Camera => {
                            eprintln!("Saving to camera is not implemented.");
                        }
                    }
                }
                PostProcessingStep::Return => {
                    if !ensure_base_tiff(&mut processed_raw, &mut base_tiff_exists)? {
                        return Err(CameraError::RsRawUtilError("No image available to return.".to_string()));
                    }
                    
                    let src = base_tiff_path_cloned.clone();
                    let jpg_data = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, CameraError> {
                        let img = image::open(&src).map_err(|e| CameraError::RsRawUtilError(format!("Failed to open base tiff: {}", e)))?;
                        let mut buffer = std::io::Cursor::new(Vec::new());
                        img.write_to(&mut buffer, image::ImageFormat::Jpeg).map_err(|e| CameraError::RsRawUtilError(format!("Failed to convert image to JPEG: {}", e)))?;
                        Ok(buffer.into_inner())
                    }).await??;

                    let image = Image::new(jpg_data, "image/jpeg".to_string(), ImageSource::Capture);
                    let _ = msg2client_sender.send(MessageToClient::Image(image));
                }
                PostProcessingStep::Upload(upload_settings) => {
                    if !ensure_base_tiff(&mut processed_raw, &mut base_tiff_exists)? {
                        return Err(CameraError::RsRawUtilError("No image available for upload.".to_string()));
                    }

                    match &upload_settings.upload_destination {
                        UploadDestination::Webdav(webdav_settings) => {
                            let mut entries = tokio::fs::read_dir(&temp_folder).await?;
                            let client = reqwest::Client::new();

                            while let Some(entry) = entries.next_entry().await? {
                                let path = entry.path();
                                if path.is_file() {
                                    let file_name = entry.file_name();
                                    let file_name_str = file_name.to_string_lossy();
                                    let url = if webdav_settings.base_url.ends_with('/') {
                                        format!("{}{}", webdav_settings.base_url, file_name_str)
                                    } else {
                                        format!("{}/{}", webdav_settings.base_url, file_name_str)
                                    };

                                    let file_content = tokio::fs::read(&path).await?;
                                    let mut request = client.put(&url).body(file_content);

                                    if let (Some(user), Some(pass)) = (&webdav_settings.username, &webdav_settings.password) {
                                        request = request.basic_auth(user, Some(pass));
                                    }

                                    match request.send().await {
                                        Ok(response) => {
                                            if !response.status().is_success() {
                                                return Err(CameraError::RsRawUtilError(format!("Failed to upload {} to WebDAV: HTTP {}", file_name_str, response.status())));
                                            } else {
                                                println!("Successfully uploaded {} to WebDAV", file_name_str);
                                            }
                                        }
                                        Err(e) => {
                                            return Err(CameraError::RsRawUtilError(format!("Error uploading {} to WebDAV: {}", file_name_str, e)));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}