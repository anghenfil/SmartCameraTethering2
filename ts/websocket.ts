import {
    WebSocketMsgType,
    MessageToServer, ImageInfo, WebSocketMessage
} from "./scheme";

let socket: WebSocket | undefined;
let url: string = `ws://${window.location.host}/ws`;
let lastImageInfo: ImageInfo | undefined;

export let onMessage: ((msg: WebSocketMessage) => void) | undefined;

export function setUrl(newUrl: string) {
    url = newUrl;
}

export async function connect(): Promise<void> {
    return new Promise((resolve, reject) => {
        socket = new WebSocket(url);
        socket.binaryType = "arraybuffer";

        socket.onopen = () => {
            console.log("WebSocket connected");
            resolve();
        };

        socket.onmessage = (event) => {
            if (event.data instanceof ArrayBuffer) {
                handleBinaryMessage(event.data);
            } else {
                console.error("Received unexpected text message:", event.data);
            }
        };

        socket.onclose = () => {
            console.log("WebSocket disconnected, retrying in 2 seconds...");
            setTimeout(() => connect(), 2000);
        };

        socket.onerror = (error) => {
            console.error("WebSocket error:", error);
            reject(error);
        };
    });
}

function handleBinaryMessage(buffer: ArrayBuffer) {
    const data = new Uint8Array(buffer);
    if (data.length === 0) return;

    const type = data[0];
    const payload = data.slice(1);

    if (type === WebSocketMsgType.JSON) {
        const text = new TextDecoder().decode(payload);
        try {
            const msg: any = JSON.parse(text);
            if (msg.id && msg.mime_type && msg.source) {
                lastImageInfo = msg;
            } else {
                if (onMessage) onMessage(msg);
            }
        } catch (e) {
            console.error("Error parsing JSON message:", e, text);
        }
    } else if (type === WebSocketMsgType.BinaryImage) {
        if (lastImageInfo) {
            if (onMessage) {
                onMessage({
                    Image: {
                        ...lastImageInfo,
                        data: payload
                    }
                });
            }
            lastImageInfo = undefined;
        } else {
            console.warn("Received binary image without preceding JSON info");
        }
    } else {
        console.warn("Unknown message type:", type);
    }
}

export function send(msg: MessageToServer) {
    if (!socket || socket.readyState !== WebSocket.OPEN) {
        console.error("Cannot send message: WebSocket is not open. Trying to reconnect...");
        connect().catch(error => console.error("Failed to reconnect:", error));
        return;
    }

    const json = JSON.stringify(msg);
    const encoder = new TextEncoder();
    const payload = encoder.encode(json);

    // Rust server socket_reader doesn't expect prefix byte for incoming messages.
    socket.send(payload);
}

export function setOnMessage(handler: (msg: WebSocketMessage) => void) { onMessage = handler; }
