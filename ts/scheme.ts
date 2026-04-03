// First byte of websocket messages
export enum WebSocketMsgType {
    JSON = 10,
    BinaryImage = 20,
}

export interface ConnectToCamera {
    port?: string;
    model?: string;
}

export type ConfigValue = string | number | boolean | "ButtonPress";

export interface ConfigUpdate {
    key: string;
    value: ConfigValue;
}

export interface UpdateCameraConfig {
    config_updates: ConfigUpdate[];
}

export interface IntervalCapture {
    number_of_images?: number;
    max_capture_time_ms?: number;
    interval_ms: number;
}

export type BlendingMode = "Additive" | "Average" | "Bright" | "Dark" | "PreferChanged";

export interface MultipleExposureCapture {
    number_of_images: number;
    interval_ms: number;
    blending_mode: BlendingMode;
}

export type CaptureImage = "Single" | { Interval: IntervalCapture } | { MultipleExposure: MultipleExposureCapture };

export type OutputFormat = "JPEG" | "PNG" | "TIFF" | "RAW";

export interface BlendImages {
    number_of_images: number;
    blending_mode: BlendingMode;
}

export type OutputDestination = "Camera" | "SystemStorage";

export interface ConvertImage {
    output_formats: OutputFormat[];
}

export interface SaveImage {
    output_destination: OutputDestination;
}

export interface WebdavUploadDestination {
    base_url: string;
    username?: string;
    password?: string;
}

export type UploadDestination = { Webdav: WebdavUploadDestination };

export interface UploadImage {
    upload_destination: UploadDestination;
}

export type PostProcessingStep = 
    | { Blend: BlendImages }
    | { Convert: ConvertImage }
    | { Save: SaveImage } 
    | "Return"
    | { Upload: UploadImage };

export interface PostProcessingConfig {
    steps: PostProcessingStep[];
    trigger_every_n_images: number;
}

export type MessageToServer =
    | "GetDetectedCameras"
    | "GetCameraStatus"
    | { ConnectToCamera: ConnectToCamera }
    | "GetCameraConfig"
    | { UpdateCameraConfig: UpdateCameraConfig }
    | { CaptureImage: CaptureImage }
    | "CapturePreviewImage"
    | "CancelCapture"
    | { SetPostProcessingConfigs: PostProcessingConfig[] };

export interface AvailableFolderOperations {
    delete_all: boolean;
    put_file: boolean;
    make_dir: boolean;
    remove_dir: boolean;
}

export interface AvailableFileOperations {
    delete: boolean;
    preview: boolean;
    raw: boolean;
    audio: boolean;
    exif: boolean;
}

export interface AvailableCameraOperations {
    capture_image: boolean;
    capture_video: boolean;
    capture_audio: boolean;
    capture_preview: boolean;
    configure: boolean;
    trigger_capture: boolean;
}

export interface CameraStatus {
    connected: boolean;
    camera_id?: string;
    model?: string;
    port?: string;
    about?: string;
    summary?: string;
    available_folder_operations?: AvailableFolderOperations;
    available_file_operations?: AvailableFileOperations;
    available_camera_operations?: AvailableCameraOperations;
}

export interface CameraDescriptor {
    model: string;
    port: string;
}

export interface CameraList {
    cameras: CameraDescriptor[];
}

export interface ValidConfigValueRange {
    min: number;
    max: number;
    step: number;
}

export type ValidConfigValues =
    | { List: string[] }
    | { Range: ValidConfigValueRange }
    | "Date"
    | "ButtonPress"
    | "Toggle"
    | "Text";

export interface ConfigSetting {
    name: string;
    label?: string;
    info?: string;
    readonly: boolean;
    value: ConfigValue;
    accepted_values: ValidConfigValues;
}

export interface CameraConfig {
    values: { [key: string]: ConfigSetting };
}

export interface Image {
    id: string;
    mime_type: string;
    data: Uint8Array;
}

export interface ImageInfo {
    id: string;
    mime_type: string;
    source: ImageSource
}

export type ImageSource =
    | "Preview"
    | "Capture"

export type CameraError =
    | { Gphoto2Error: string }
    | { IoError: string }
    | { TokioError: string }
    | { CaptureTookLongerThanInterval: number }


export type MessageToClient =
    | { CameraStatus: CameraStatus }
    | { CameraList: CameraList }
    | { CameraConfig: CameraConfig }
    | { Image: ImageInfo } // This is what comes via JSON message before binary
    | { PreviewImage: ImageInfo}
    | { ImageList: ImageInfo[] }
    | { CameraError: CameraError}
    | "CaptureCancelled"
    | "CaptureComplete";


export type WebSocketMessage = MessageToClient | { Image: Image };
