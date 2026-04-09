use crate::camera::worker::CameraError;
use gphoto2::abilities::FolderOperations;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use rsraw_utils::blending::BlendingMode;
use SmartCameraTethering2_shared_types::{PostProcessingConfig as SharedPostProcessingConfig, PostProcessingStep};

/// Local wrapper for PostProcessingConfig that includes transport-only fields
/// (compress, compression_level) which are irrelevant to the post-processing server.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PostProcessingConfig {
    pub steps: Vec<PostProcessingStep>,
    pub trigger_every_n_images: u32,
    #[serde(default)]
    pub compress: bool,
    #[serde(default = "default_compression_level")]
    pub compression_level: u8,
}

fn default_compression_level() -> u8 { 1 }

impl From<PostProcessingConfig> for SharedPostProcessingConfig {
    fn from(c: PostProcessingConfig) -> Self {
        SharedPostProcessingConfig {
            steps: c.steps,
            trigger_every_n_images: c.trigger_every_n_images,
        }
    }
}

// first byte of websocket messages
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum WebSocketMsgType {
    JSON = 10,
    BinaryImage = 20,
}

impl From<WebSocketMsgType> for u8 {
    fn from(msg_type: WebSocketMsgType) -> Self {
        msg_type as u8
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ConnectToCamera {
    pub port: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum ConfigValue {
    Str(String),
    F32(f32),
    Bool(bool),
    ButtonPress,
    Int(i64),
}

impl From<String> for ConfigValue {
    fn from(value: String) -> Self {
        ConfigValue::Str(value)
    }
}

impl From<f32> for ConfigValue {
    fn from(value: f32) -> Self {
        ConfigValue::F32(value)
    }
}

impl From<bool> for ConfigValue {
    fn from(value: bool) -> Self {
        ConfigValue::Bool(value)
    }
}

impl From<i64> for ConfigValue {
    fn from(value: i64) -> Self {
        ConfigValue::Int(value)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConfigUpdate {
    pub key: String,
    pub value: ConfigValue,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UpdateCameraConfig {
    pub config_updates: Vec<ConfigUpdate>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IntervalCapture {
    pub number_of_images: Option<u32>,
    pub max_capture_time_ms: Option<u32>,
    pub interval_ms: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MultipleExposureCapture {
    pub number_of_images: u32,
    pub interval_ms: u32,
    pub blending_mode: BlendingMode,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum CaptureImage {
    Single,
    Interval(IntervalCapture),
    MultipleExposure(MultipleExposureCapture),
}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum MessageToServer {
    GetDetectedCameras,
    GetCameraStatus,
    ConnectToCamera(ConnectToCamera),
    GetCameraConfig,
    UpdateCameraConfig(UpdateCameraConfig),
    CaptureImage(CaptureImage),
    CapturePreviewImage,
    CancelCapture,
    SetPostProcessingConfigs(Vec<PostProcessingConfig>),
    Shutdown,
    DisconnectCamera,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum MessageToClient {
    CameraStatus(CameraStatus),
    CameraList(CameraList),
    CameraConfig(CameraConfig),
    Image(Image),
    ImageList(Vec<Image>),
    CameraError(CameraError),
    CaptureCancelled,
    CaptureComplete,
}

impl From<CameraStatus> for MessageToClient {
    fn from(value: CameraStatus) -> Self {
        MessageToClient::CameraStatus(value)
    }
}

impl From<CameraList> for MessageToClient {
    fn from(value: CameraList) -> Self {
        MessageToClient::CameraList(value)
    }
}

impl From<CameraConfig> for MessageToClient {
    fn from(value: CameraConfig) -> Self {
        MessageToClient::CameraConfig(value)
    }
}

impl From<Image> for MessageToClient {
    fn from(value: Image) -> Self {
        MessageToClient::Image(value)
    }
}

impl From<Vec<Image>> for MessageToClient {
    fn from(value: Vec<Image>) -> Self {
        MessageToClient::ImageList(value)
    }
}

impl<T> From<Result<T, CameraError>> for MessageToClient
where
    MessageToClient: From<T>,
{
    fn from(value: Result<T, CameraError>) -> Self {
        match value {
            Err(e) => MessageToClient::CameraError(e),
            Ok(value) => MessageToClient::from(value),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AvailableFolderOperations {
    pub delete_all: bool,
    pub put_file: bool,
    pub make_dir: bool,
    pub remove_dir: bool,
}

impl From<FolderOperations> for AvailableFolderOperations {
    fn from(folder_operations: FolderOperations) -> Self {
        AvailableFolderOperations {
            delete_all: folder_operations.delete_all(),
            put_file: folder_operations.put_file(),
            make_dir: folder_operations.make_dir(),
            remove_dir: folder_operations.remove_dir(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AvailableFileOperations {
    pub delete: bool,
    pub preview: bool,
    pub raw: bool,
    pub audio: bool,
    pub exif: bool,
}

impl From<gphoto2::abilities::FileOperations> for AvailableFileOperations {
    fn from(file_operations: gphoto2::abilities::FileOperations) -> Self {
        AvailableFileOperations {
            delete: file_operations.delete(),
            preview: file_operations.preview(),
            raw: file_operations.raw(),
            audio: file_operations.audio(),
            exif: file_operations.exif(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AvailableCameraOperations {
    pub capture_image: bool,
    pub capture_video: bool,
    pub capture_audio: bool,
    pub capture_preview: bool,
    pub configure: bool,
    pub trigger_capture: bool,
}

impl From<gphoto2::abilities::CameraOperations> for AvailableCameraOperations {
    fn from(camera_operations: gphoto2::abilities::CameraOperations) -> Self {
        AvailableCameraOperations {
            capture_image: camera_operations.capture_image(),
            capture_video: camera_operations.capture_video(),
            capture_audio: camera_operations.capture_audio(),
            capture_preview: camera_operations.capture_preview(),
            configure: camera_operations.configure(),
            trigger_capture: camera_operations.trigger_capture(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CameraStatus {
    pub connected: bool,
    pub camera_id: Option<String>,
    pub model: Option<String>,
    pub port: Option<String>,
    pub about: Option<String>,
    pub summary: Option<String>,
    pub available_folder_operations: Option<AvailableFolderOperations>,
    pub available_file_operations: Option<AvailableFileOperations>,
    pub available_camera_operations: Option<AvailableCameraOperations>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CameraDescriptor {
    pub model: String,
    pub port: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CameraList {
    pub cameras: Vec<CameraDescriptor>,
}

impl From<Vec<CameraDescriptor>> for CameraList {
    fn from(cameras: Vec<CameraDescriptor>) -> Self {
        Self { cameras }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ValidConfigValueRange {
    pub min: f32,
    pub max: f32,
    pub step: f32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum ValidConfigValues {
    List(Vec<String>),
    Range(ValidConfigValueRange),
    Date,
    ButtonPress,
    Toggle,
    Text,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConfigSetting {
    pub name: String,
    pub label: Option<String>,
    pub info: Option<String>,
    pub readonly: bool,
    pub value: ConfigValue,
    pub accepted_values: ValidConfigValues,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CameraConfig {
    pub values: BTreeMap<String, ConfigSetting>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Image {
    pub id: String,
    pub mime_type: String,
    pub source: ImageSource,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ImageSource {
    Preview,
    Capture,
    PostProcessing,
}

impl Image {
    pub fn new(data: Vec<u8>, mime_type: String, source: ImageSource) -> Self {
        Image {
            id: uuid::Uuid::new_v4().to_string(),
            mime_type,
            source,
            data,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ImageInfo {
    pub id: String,
    pub mime_type: String,
    pub source: ImageSource,
}

impl Image {
    pub fn split(self) -> (ImageInfo, Vec<u8>) {
        (
            ImageInfo {
                id: self.id,
                mime_type: self.mime_type,
                source: self.source,
            },
            self.data,
        )
    }
}
