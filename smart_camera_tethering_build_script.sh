#!/bin/bash
set -e

IMAGE_NAME="smartcamera-arm64-builder"

# Image bauen (nur beim ersten Mal oder bei Änderungen)
docker build --platform linux/arm64 -t $IMAGE_NAME -f Dockerfile.arm64 .

# Projekt kompilieren
docker run --rm --platform linux/arm64 \
    -v "$(pwd)":/project \
    -v cargo-cache-arm64:/usr/local/cargo/registry \
    $IMAGE_NAME \
    bash -c "npm install && cargo update && cargo build --release"

echo "Binary liegt unter: target/release/smart_camera_tethering2"
