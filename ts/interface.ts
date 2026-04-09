import {
    CameraConfig, CameraError,
    CameraList,
    CameraStatus,
    ConfigSetting,
    ConnectToCamera,
    Image, IntervalCapture, MessageToServer, PostProcessingConfig, PostProcessingStep, UpdateCameraConfig,
    ValidConfigValues
} from "./scheme";
import * as WebSocketClient from './websocket';
import {camera_config, set_is_capturing} from "./index";

export function update_available_cameras(list: CameraList){
    let select = document.getElementById("availableCameras") as HTMLSelectElement;
    if(list.cameras.length == 0){
        select.innerHTML = "<option selected>No cameras available</option>";
        return;
    }
    select.innerHTML = "";
    for(let camera of list.cameras){
        select.innerHTML += `<option data-camera-model="${camera.model}" data-camera-port="${camera.port}">${camera.model} @ ${camera.port}</option>`;
    }
}

export function add_interface_event_listeners(){
    document.getElementById("connect")?.addEventListener("click", connect_to_camera);
    document.getElementById("disconnect")?.addEventListener("click", () => {
        WebSocketClient.send("DisconnectCamera");
    });
    document.getElementById("startPreview")?.addEventListener("click", start_preview);
    document.getElementById("stopPreview")?.addEventListener("click", stop_preview);
    document.getElementById("shutdown")?.addEventListener("click", () => {
        if (confirm("Are you sure you want to shutdown the Raspberry Pi?")) {
            WebSocketClient.send("Shutdown");
        }
    });
    add_capture_listener();
    add_cancel_capture_listener();
    add_post_processing_listeners();
    update_capture_settings_options();
}

export function connect_to_camera(e: Event){
    let select = document.getElementById("availableCameras") as HTMLSelectElement;
    let selected = select.selectedOptions[0];

    if(selected.getAttribute("data-camera-model") == null || selected.getAttribute("data-camera-port") == null){
        console.error("No camera selected");
        alert("Please select a camera to connect to.");
        return;
    }

    let port = selected.getAttribute("data-camera-port") as string;
    let model = selected.getAttribute("data-camera-model") as string;
    let settings : ConnectToCamera = {
        port: port,
        model: model
    }

    console.log("Connecting to camera:", settings);

    WebSocketClient.send({ ConnectToCamera: settings });
}

export function show_camera_status(status: CameraStatus){
    console.log("Camera status:", status);

    let camera_status = document.getElementById("cameraStatus") as HTMLElement;
    let connect_btn = document.getElementById("connect") as HTMLElement;
    let disconnect_btn = document.getElementById("disconnect") as HTMLElement;

    if(status.connected){
        let details = "("+status.model+"@"+status.port+")";
        camera_status.innerText = "Connected "+details;
        connect_btn.classList.add("hide");
        disconnect_btn.classList.remove("hide");
    }else{
        camera_status.innerText = "Disconnected";
        connect_btn.classList.remove("hide");
        disconnect_btn.classList.add("hide");
    }
}

let preview_interval: NodeJS.Timeout | undefined;

function start_preview(){
    document.getElementById("startPreview")?.classList.add("hide");
    document.getElementById("stopPreview")?.classList.remove("hide");
    preview_interval = setInterval(() => {
        WebSocketClient.send("CapturePreviewImage")
    }, 200);
}

function stop_preview(){
    document.getElementById("startPreview")?.classList.remove("hide");
    document.getElementById("stopPreview")?.classList.add("hide");
    document.getElementById("image")?.setAttribute("src", "");
    clearInterval(preview_interval);
}

const MAX_IMAGES = 3;
let image_buffer: { url: string; source: string }[] = [];
let image_index: number = 0;

function source_label(source: string): string {
    if (source === "Preview") return "Preview";
    if (source === "Capture") return "Capture";
    if (source === "PostProcessing") return "Post-Processing";
    return source;
}

function render_current_image() {
    const img = document.getElementById("image") as HTMLImageElement;
    const sourceLabel = document.getElementById("imageSource") as HTMLSpanElement;
    const prevBtn = document.getElementById("imagePrev") as HTMLButtonElement;
    const nextBtn = document.getElementById("imageNext") as HTMLButtonElement;

    if (image_buffer.length === 0) return;

    const entry = image_buffer[image_index];
    img.src = entry.url;
    sourceLabel.textContent = `${source_label(entry.source)} (${image_index + 1}/${image_buffer.length})`;
    prevBtn.disabled = image_index <= 0;
    nextBtn.disabled = image_index >= image_buffer.length - 1;
}

function setup_image_nav_listeners() {
    const prevBtn = document.getElementById("imagePrev") as HTMLButtonElement;
    const nextBtn = document.getElementById("imageNext") as HTMLButtonElement;
    prevBtn.addEventListener("click", () => {
        if (image_index > 0) {
            image_index--;
            render_current_image();
        }
    });
    nextBtn.addEventListener("click", () => {
        if (image_index < image_buffer.length - 1) {
            image_index++;
            render_current_image();
        }
    });
}
setup_image_nav_listeners();

export function show_image(image: Image){
    console.log("Showing image.");
    const blob = new Blob([image.data.buffer as ArrayBuffer], {type: image.mime_type});
    const url = URL.createObjectURL(blob);

    if (image_buffer.length === MAX_IMAGES) {
        URL.revokeObjectURL(image_buffer[0].url);
        image_buffer.shift();
    }
    image_buffer.push({ url, source: image.source as string });
    image_index = image_buffer.length - 1;

    render_current_image();
}

export function show_camera_config(config: CameraConfig){
    console.log("Camera config:", config);
    let camera_config = document.getElementById("cameraSettings") as HTMLElement;

    camera_config.innerHTML = "";
    for(let setting_id in config.values){
        let setting = config.values[setting_id];
        camera_config.innerHTML += `<div class="row setting_row" data-setting="${setting_id}"><span class="col">${setting.label}</span><span class="col">${setting.value}</span></div>`
    }

    update_capture_settings_options();
}


let capture_interval_checkbox = document.getElementById("capture_interval") as HTMLInputElement;
let capture_interval_settings = document.getElementById("capture_interval_settings") as HTMLElement;
let capture_number_of_shots = document.getElementById("capture_number_of_shots") as HTMLInputElement;
let capture_interval_time = document.getElementById("capture_interval_time") as HTMLInputElement;
let bulb_exposure_time_group = document.getElementById("bulb_exposure_time_container") as HTMLElement;
let iso_select = document.getElementById("capture_iso") as HTMLSelectElement;
let shutter_speed_select = document.getElementById("capture_shutter_speed") as HTMLSelectElement;
let aperture_select = document.getElementById("capture_aperture") as HTMLSelectElement;
let focus_mode_select = document.getElementById("capture_focus_mode") as HTMLSelectElement;
let exposure_compensation = document.getElementById("capture_exposure_compensation") as HTMLSelectElement;

function update_capture_settings_options(){
    if(camera_config == undefined){
        return;
    }

    if(capture_interval_checkbox.checked){
        capture_interval_settings.classList.remove("hide");
    }else{
        capture_interval_settings.classList.add("hide");
    }
    capture_interval_checkbox.addEventListener("change", (e) => {
        if(capture_interval_checkbox.checked){
            capture_interval_settings.classList.remove("hide");
        }else{
            capture_interval_settings.classList.add("hide");
        }
    });

    let iso_config = camera_config.values["main.imgsettings.iso"];
    if(iso_config){
        generate_options_for_config_setting(iso_config, iso_select);
        iso_select.disabled = iso_config.readonly;
        iso_select.addEventListener("change", update_capture_setting_listener);
    }
    let shutter_speed_config = camera_config.values["main.capturesettings.shutterspeed"];
    if(shutter_speed_config){
        generate_options_for_config_setting(shutter_speed_config, shutter_speed_select);
        shutter_speed_select.disabled = shutter_speed_config.readonly;
        shutter_speed_select.addEventListener("change", update_capture_setting_listener);
        shutter_speed_select.addEventListener("change", (e) => {
            let selected_option = shutter_speed_select.selectedOptions[0];
            if(selected_option.value == "Bulb"){
                bulb_exposure_time_group.classList.remove("hide");
            }else{
                bulb_exposure_time_group.classList.add("hide");
            }
        })
    }
    let aperture_config = camera_config.values["main.capturesettings.f-number"];
    if(aperture_config){
        generate_options_for_config_setting(aperture_config, aperture_select);
        aperture_select.disabled = aperture_config.readonly;
        aperture_select.addEventListener("change", update_capture_setting_listener);
    }
    let focus_mode_config = camera_config.values["main.capturesettings.focusmode"];
    if(focus_mode_config){
        generate_options_for_config_setting(focus_mode_config, focus_mode_select);
        focus_mode_select.disabled = focus_mode_config.readonly;
        focus_mode_select.addEventListener("change", update_capture_setting_listener);
    }
    let exposure_compensation_config = camera_config.values["main.capturesettings.exposurecompensation"];
    if(exposure_compensation_config){
        generate_options_for_config_setting(exposure_compensation_config, exposure_compensation);
        exposure_compensation.disabled = exposure_compensation_config.readonly;
        exposure_compensation.addEventListener("change", update_capture_setting_listener);
    }
}

function generate_options_for_config_setting(setting: ConfigSetting, select: HTMLSelectElement){
    if (typeof setting.accepted_values === "object" && "List" in setting.accepted_values){
        let list = setting.accepted_values.List;
        select.innerHTML = "";

        for(let value of list){
            let option = new Option(value, value, false, value === setting.value);

            select.add(option);
        }

    }else{
        throw new Error("Expected setting.accepted_values to be a List, got: " + typeof setting.accepted_values);
    }
}

function update_capture_setting_listener(e: Event){
    let target = e.target;

    if(target instanceof HTMLSelectElement){
        let selected_option = target.selectedOptions[0];
        let setting_id = target.getAttribute("data-setting-name");
        if(setting_id){
            let change : UpdateCameraConfig = {
                config_updates: [
                    {
                        key: setting_id,
                        value: selected_option.value
                    }
                ]
            }
            let msg : MessageToServer = {
                UpdateCameraConfig: change
            }
            try{
                WebSocketClient.send(msg);
                show_alert("Updating capture setting", "info");
            }catch(e){
                console.error("Failed to update capture setting:", e);
                // Refresh settings
                update_capture_settings_options();
            }
        }
    }else if(target instanceof HTMLInputElement){

    }
}

function add_capture_listener(){
    let btn = document.getElementById("capture") as HTMLButtonElement;

    btn.addEventListener("click", () => {
        set_is_capturing(true);
        if(capture_interval_checkbox.checked){
            // Interval capture
            let interval_settings: IntervalCapture = {
                number_of_images: parseInt(capture_number_of_shots.value),
                interval_ms: (parseInt(capture_interval_time.value)*1000)
            }
            WebSocketClient.send({
                "CaptureImage": { Interval: interval_settings }
            })
        }else{
            // Single capture
            WebSocketClient.send({
                "CaptureImage": "Single"
            });
        }
    });
}

function add_cancel_capture_listener() {
    let btn = document.getElementById("cancel_capture") as HTMLButtonElement;
    btn.addEventListener("click", () => {
        WebSocketClient.send("CancelCapture");
    });
}

function add_post_processing_listeners() {
    document.getElementById("add_post_processing_config")?.addEventListener("click", () => {
        add_post_processing_config_ui();
        apply_post_processing();
    });
}

function add_post_processing_config_ui(config?: PostProcessingConfig) {
    const container = document.getElementById("post_processing_configs") as HTMLElement;
    const configDiv = document.createElement("div");
    configDiv.className = "card mb-3 post-processing-config";
    configDiv.innerHTML = `
        <div class="card-header d-flex justify-content-between align-items-center">
            <span>Config</span>
            <button class="btn btn-sm btn-danger remove-config">Remove</button>
        </div>
        <div class="card-body">
            <div class="input-group mb-3">
                <span class="input-group-text">Trigger Every N Images</span>
                <input type="number" class="form-control trigger-every" min="1" value="${config?.trigger_every_n_images || 1}">
            </div>
            <div class="form-check mb-3">
                <input class="form-check-input compress-raw" type="checkbox" ${config?.compress ? 'checked' : ''}>
                <label class="form-check-label">Compress RAW before sending (zstd)</label>
            </div>
            <h6>Steps</h6>
            <div class="steps-container"></div>
            <button class="btn btn-sm btn-outline-secondary add-step">Add Step</button>
        </div>
    `;

    configDiv.querySelector(".remove-config")?.addEventListener("click", () => {
        configDiv.remove();
        apply_post_processing();
    });
    configDiv.querySelector(".add-step")?.addEventListener("click", () => {
        add_step_ui(configDiv.querySelector(".steps-container")!);
        apply_post_processing();
    });

    const triggerInput = configDiv.querySelector(".trigger-every") as HTMLInputElement;
    triggerInput.addEventListener("change", apply_post_processing);
    triggerInput.addEventListener("input", apply_post_processing);

    const compressInput = configDiv.querySelector(".compress-raw") as HTMLInputElement;
    compressInput.addEventListener("change", apply_post_processing);

    if (config) {
        config.steps.forEach(step => add_step_ui(configDiv.querySelector(".steps-container")!, step));
    } else {
        // Default to one step if adding manually
        add_step_ui(configDiv.querySelector(".steps-container")!);
    }

    container.appendChild(configDiv);
}

function add_step_ui(container: Element, step?: PostProcessingStep) {
    const stepDiv = document.createElement("div");
    stepDiv.className = "card mb-2 post-processing-step";
    
    let stepType = "Return";
    if (step) {
        if (typeof step === "string") {
            if (step === "Return") stepType = "Return";
        } else {
            if ("Blend" in step) stepType = "Blend";
            else if ("Save" in step) stepType = "Save";
            else if ("Upload" in step) stepType = "Upload";
        }
    }

    stepDiv.innerHTML = `
        <div class="card-body p-2">
            <div class="d-flex justify-content-between mb-2">
                <select class="form-select form-select-sm step-type" style="width: auto;">
                    <option value="Return" ${stepType === "Return" ? "selected" : ""}>Return</option>
                    <option value="Save" ${stepType === "Save" ? "selected" : ""}>Save</option>
                    <option value="Blend" ${stepType === "Blend" ? "selected" : ""}>Blend</option>
                    <option value="Upload" ${stepType === "Upload" ? "selected" : ""}>Upload</option>
                </select>
                <button class="btn btn-sm btn-outline-danger remove-step">×</button>
            </div>
            <div class="step-details"></div>
        </div>
    `;

    const typeSelect = stepDiv.querySelector(".step-type") as HTMLSelectElement;
    const detailsContainer = stepDiv.querySelector(".step-details") as HTMLElement;

    typeSelect.addEventListener("change", () => {
        update_step_details(detailsContainer, typeSelect.value);
        apply_post_processing();
    });
    stepDiv.querySelector(".remove-step")?.addEventListener("click", () => {
        stepDiv.remove();
        apply_post_processing();
    });

    update_step_details(detailsContainer, typeSelect.value, step);
    container.appendChild(stepDiv);
}

function update_step_details(container: HTMLElement, type: string, step?: PostProcessingStep) {
    container.innerHTML = "";
    if (type === "Return") {
        container.innerHTML = `
            <div class="text-muted small">Always returns JPEG</div>
        `;
    } else if (type === "Save") {
        container.innerHTML = `
            <div class="input-group input-group-sm mb-1">
                <span class="input-group-text">Dest</span>
                <select class="form-select output-destination">
                    <option value="SystemStorage">System Storage</option>
                    <option value="Camera">Camera</option>
                    <option value="ServerStorage">Server Storage</option>
                </select>
            </div>
            <div class="input-group input-group-sm server-storage-path-group" style="display:none">
                <span class="input-group-text">Path</span>
                <input type="text" class="form-control server-storage-path" placeholder="subdir/name">
            </div>
        `;
        const destSelect = container.querySelector(".output-destination") as HTMLSelectElement;
        const pathGroup = container.querySelector(".server-storage-path-group") as HTMLElement;
        const pathInput = container.querySelector(".server-storage-path") as HTMLInputElement;
        const togglePathGroup = () => {
            pathGroup.style.display = destSelect.value === "ServerStorage" ? "" : "none";
        };
        if (step && typeof step !== "string" && "Save" in step) {
            const dest = step.Save.output_destination;
            if (typeof dest === "object" && "ServerStorage" in dest) {
                destSelect.value = "ServerStorage";
                pathInput.value = dest.ServerStorage;
            } else {
                destSelect.value = dest as string;
            }
        }
        togglePathGroup();
        destSelect.addEventListener("change", () => { togglePathGroup(); apply_post_processing(); });
        pathInput.addEventListener("change", apply_post_processing);
        pathInput.addEventListener("input", apply_post_processing);
    } else if (type === "Blend") {
        container.innerHTML = `
            <div class="input-group input-group-sm mb-1">
                <span class="input-group-text">Num Images</span>
                <input type="number" class="form-control blend-num" min="2" value="2">
            </div>
            <div class="input-group input-group-sm">
                <span class="input-group-text">Mode</span>
                <select class="form-select blend-mode">
                    <option value="Average">Average</option>
                    <option value="Additive">Additive</option>
                    <option value="Bright">Bright</option>
                    <option value="Dark">Dark</option>
                    <option value="PreferChanged">Prefer Changed</option>
                </select>
            </div>
        `;
        if (step && typeof step !== "string" && "Blend" in step) {
            (container.querySelector(".blend-num") as HTMLInputElement).value = step.Blend.number_of_images.toString();
            (container.querySelector(".blend-mode") as HTMLSelectElement).value = step.Blend.blending_mode;
        }
        const blendNum = container.querySelector(".blend-num") as HTMLInputElement;
        blendNum.addEventListener("change", apply_post_processing);
        blendNum.addEventListener("input", apply_post_processing);
        container.querySelector(".blend-mode")?.addEventListener("change", apply_post_processing);
    } else if (type === "Upload") {
        container.innerHTML = `
            <div class="input-group input-group-sm mb-1">
                <span class="input-group-text">URL</span>
                <input type="text" class="form-control upload-url" placeholder="http://..." value="http://">
            </div>
            <div class="input-group input-group-sm mb-1">
                <span class="input-group-text">User</span>
                <input type="text" class="form-control upload-user">
            </div>
             <div class="input-group input-group-sm">
                <span class="input-group-text">Pass</span>
                <input type="password" class="form-control upload-pass">
            </div>
        `;
        if (step && typeof step !== "string" && "Upload" in step) {
            (container.querySelector(".upload-url") as HTMLInputElement).value = step.Upload.upload_destination.Webdav.base_url;
            (container.querySelector(".upload-user") as HTMLInputElement).value = step.Upload.upload_destination.Webdav.username || "";
            (container.querySelector(".upload-pass") as HTMLInputElement).value = step.Upload.upload_destination.Webdav.password || "";
        }
        const inputs = container.querySelectorAll("input");
        inputs.forEach(input => {
            input.addEventListener("change", apply_post_processing);
            input.addEventListener("input", apply_post_processing);
        });
    }
}

function apply_post_processing() {
    const configs: PostProcessingConfig[] = [];
    const configDivs = document.querySelectorAll(".post-processing-config");
    
    configDivs.forEach(configDiv => {
        const triggerEvery = parseInt((configDiv.querySelector(".trigger-every") as HTMLInputElement).value);
        const steps: PostProcessingStep[] = [];
        const stepDivs = configDiv.querySelectorAll(".post-processing-step");

        stepDivs.forEach(stepDiv => {
            const type = (stepDiv.querySelector(".step-type") as HTMLSelectElement).value;
            if (type === "Return") {
                steps.push("Return");
            } else if (type === "Save") {
                const destVal = (stepDiv.querySelector(".output-destination") as HTMLSelectElement).value;
                let dest: any;
                if (destVal === "ServerStorage") {
                    const path = (stepDiv.querySelector(".server-storage-path") as HTMLInputElement).value || "";
                    dest = { ServerStorage: path };
                } else {
                    dest = destVal;
                }
                steps.push({ Save: { output_destination: dest } });
            } else if (type === "Blend") {
                const num = parseInt((stepDiv.querySelector(".blend-num") as HTMLInputElement).value);
                const mode = (stepDiv.querySelector(".blend-mode") as HTMLSelectElement).value as any;
                steps.push({ Blend: { number_of_images: num, blending_mode: mode } });
            } else if (type === "Upload") {
                const url = (stepDiv.querySelector(".upload-url") as HTMLInputElement).value;
                const user = (stepDiv.querySelector(".upload-user") as HTMLInputElement).value;
                const pass = (stepDiv.querySelector(".upload-pass") as HTMLInputElement).value;
                steps.push({
                    Upload: {
                        upload_destination: {
                            Webdav: {
                                base_url: url,
                                username: user || undefined,
                                password: pass || undefined
                            }
                        }
                    }
                });
            }
        });

        const compress = (configDiv.querySelector(".compress-raw") as HTMLInputElement).checked;
        configs.push({
            trigger_every_n_images: triggerEvery,
            steps: steps,
            compress: compress
        });
    });

    WebSocketClient.send({ SetPostProcessingConfigs: configs });
    localStorage.setItem("post_processing_configs", JSON.stringify(configs));
}

export function load_post_processing_from_storage(): boolean {
    const saved = localStorage.getItem("post_processing_configs");
    if (saved) {
        try {
            const configs: PostProcessingConfig[] = JSON.parse(saved);
            const container = document.getElementById("post_processing_configs") as HTMLElement;
            container.innerHTML = "";
            configs.forEach(config => add_post_processing_config_ui(config));
            WebSocketClient.send({ SetPostProcessingConfigs: configs });
            return true;
        } catch (e) {
            console.error("Failed to load post-processing configs from storage:", e);
        }
    }
    return false;
}

export let alert_hider: NodeJS.Timeout | undefined;

export function update_capture_button_state(is_capturing: boolean){
    let capture_btn = document.getElementById("capture") as HTMLButtonElement;
    let cancel_btn = document.getElementById("cancel_capture") as HTMLButtonElement;

    if(is_capturing){
        capture_btn.disabled = true;
        cancel_btn.classList.remove("hide");
    }else{
        capture_btn.disabled = false;
        cancel_btn.classList.add("hide");
    }
}

export function show_alert(message: string|CameraError, level: "danger" | "warning" | "info" | "success"){
    if(typeof message != "string"){
        console.log("Error message:", message);
        if("Gphoto2Error" in message){
            message = `GPhoto2 Error: ${message.Gphoto2Error}`;
        }else if("IoError" in message){
            message = "I/O Error: "+message.IoError.toString();
        }else if("TokioError" in message){
            message = "Tokio Error: "+message.TokioError.toString();
        }else if("CaptureTookLongerThanInterval" in message){
            message = `Capture took longer than interval (${message.CaptureTookLongerThanInterval} ms)!`;
            level = "warning";
        }
    }


    let alert = document.getElementById("alert") as HTMLElement;
    alert.innerText = message;
    alert.classList = "alert alert-"+level;

    if(alert_hider){
        clearTimeout(alert_hider);
    }
    alert_hider = setTimeout(() => {
        alert.classList.add("hide");
    }, 2000);
}