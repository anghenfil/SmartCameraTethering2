use crate::camera::worker::{CameraConnection, CameraError};
use crate::websocket::scheme::{CameraStatus, ConnectToCamera};
use gphoto2::list::CameraDescriptor;
use gphoto2::{Context, context};

pub async fn connect_to_camera(port: &str, model: &str) -> Result<CameraConnection, CameraError> {
    let context = Context::new()?;
    let camera_descriptor = CameraDescriptor {
        model: model.to_string(),
        port: port.to_string(),
    };
    let camera = context.get_camera(&camera_descriptor).await?;
    Ok(CameraConnection { camera, context })
}

pub async fn autoconnect_camera() -> Result<CameraConnection, CameraError> {
    let context = context::Context::new()?;
    let camera = context.autodetect_camera().await?;
    Ok(CameraConnection { camera, context })
}

pub async fn connect(config: ConnectToCamera) -> Result<CameraConnection, CameraError> {
    if let Some(port) = &config.port
        && let Some(model) = &config.model
    {
        let _camera_descriptor = CameraDescriptor {
            model: model.to_string(),
            port: port.to_string(),
        };
        let camera = connect_to_camera(port, model).await?;
        Ok(camera)
    } else {
        Ok(autoconnect_camera().await?)
    }
}

pub async fn get_camera_status(
    connection: &mut CameraConnection,
) -> Result<CameraStatus, CameraError> {
    let abilities = connection.camera.abilities();

    let camera_status = CameraStatus {
        connected: true,
        camera_id: Some(abilities.id().to_string()),
        model: Some(abilities.model().to_string()),
        port: Some(connection.camera.port_info()?.name()),
        about: Some(connection.camera.about()?),
        summary: Some(connection.camera.summary()?),
        available_folder_operations: Some(abilities.folder_operations().into()),
        available_file_operations: Some(abilities.file_operations().into()),
        available_camera_operations: Some(abilities.camera_operations().into()),
    };

    Ok(camera_status)
}
