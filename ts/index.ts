import * as WebSocketClient from './websocket';
import {CameraConfig, MessageToServer, WebSocketMessage} from './scheme';
import {
    add_interface_event_listeners, show_camera_status, update_available_cameras, show_image, show_camera_config,
    show_alert, update_capture_button_state, load_post_processing_from_storage
} from './interface';
import deepEqual from "deep-equal";

let camera_connected = false;
let is_capturing = false;
export let camera_config: CameraConfig | undefined;

document.addEventListener("DOMContentLoaded", async function(event) {
    console.log("Connecting to websocket...");

    // 1. Set up the message handler
    WebSocketClient.setOnMessage((msg: WebSocketMessage) => {
        console.log("Received message:", msg);

        if (msg === "CaptureCancelled") {
            is_capturing = false;
            update_capture_button_state(is_capturing);
            show_alert("Capture cancelled", "success");
            return;
        }

        if (msg === "CaptureComplete") {
            is_capturing = false;
            update_capture_button_state(is_capturing);
            show_alert("Capture complete", "success");
            return;
        }

        if (typeof msg !== "object") {
            console.log("Unhandled message", msg);
            return;
        }

        if ('CameraList' in msg) {
            console.log("Available cameras:", msg.CameraList.cameras);
            update_available_cameras(msg.CameraList);
        } else if('CameraStatus' in msg){
            camera_connected = msg.CameraStatus.connected;
            show_camera_status(msg.CameraStatus);
        } else if('CameraConfig' in msg){
            if(!deepEqual(msg.CameraConfig.values, camera_config?.values)){
                console.log("Camera config changed:", msg.CameraConfig);
                camera_config = msg.CameraConfig;
                show_camera_config(msg.CameraConfig);
            }else{
                console.log("Camera config unchanged");
            }
        } else if('CameraError' in msg){
            is_capturing = false;
            update_capture_button_state(is_capturing);
            console.error("Camera error:", msg.CameraError);
            
            let errorMsg = "Unknown error";
            const err = msg.CameraError;
            if (typeof err === "string") {
                errorMsg = err;
            } else if ("Gphoto2Error" in err) {
                errorMsg = "Camera Error: " + err.Gphoto2Error;
            } else if ("IoError" in err) {
                errorMsg = "IO Error: " + err.IoError;
            } else if ("TokioError" in err) {
                errorMsg = "System Error: " + err.TokioError;
            } else if ("CaptureTookLongerThanInterval" in err) {
                errorMsg = `Capture took too long: ${err.CaptureTookLongerThanInterval}ms`;
            }
            
            show_alert(errorMsg, "danger");
        } else if('Image' in msg && 'data' in msg.Image){
            show_image(msg.Image);
        }
        else{
            console.log("Unhandled message", msg);
        }
    });

    try {
        // Connect to the server and wait for it to be open
        await WebSocketClient.connect();
        console.log("Connected!");

        console.log("Requesting camera status...");
        WebSocketClient.send("GetCameraStatus");
        const getCamerasMsg: MessageToServer = "GetDetectedCameras";
        console.log("Requesting camera list...");
        WebSocketClient.send(getCamerasMsg);
    } catch (e) {
        console.error("Failed to connect:", e);
    }

    // Add interface event listeners
    add_interface_event_listeners();

    // Load post-processing config from storage
    load_post_processing_from_storage();

    // If not connected, list available cameras every few seconds
    query_available_cameras();

    // If connected, query camera settings every 1 second
    query_settings();
});

function query_available_cameras(){
    if (!camera_connected){
        WebSocketClient.send("GetDetectedCameras");
        let interval = setInterval(() => {
            if (camera_connected){
                clearInterval(interval);
                return;
            }
            WebSocketClient.send("GetDetectedCameras");
        }, 3000
        )
    }
}

function query_settings(){
    let interval = setInterval(() => {
        if (camera_connected && !is_capturing){
            WebSocketClient.send("GetCameraConfig");
        }
    }, 2000);
}

export function set_is_capturing(capturing: boolean){
    is_capturing = capturing;
    update_capture_button_state(is_capturing);
}