#!/bin/bash
set -e

IMAGE_NAME="fac-test-builder"
CONTAINER_CARGO_REGISTRY="fac-test-cargo-registry"
CONTAINER_TARGET="fac-test-linux-target"

echo "=== 构建 Docker 编译镜像 (首次较慢, 后续秒级缓存) ==="
docker build -t "$IMAGE_NAME" -f - . <<'DOCKERFILE'
FROM ubuntu:20.04

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y \
    curl build-essential pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"
DOCKERFILE

echo "=== 开始编译 ==="
docker run --rm \
    -v "$PWD":/src \
    -v "$CONTAINER_CARGO_REGISTRY":/root/.cargo/registry \
    -v "$CONTAINER_TARGET":/build-target \
    -e CARGO_TARGET_DIR=/build-target \
    -w /src \
    "$IMAGE_NAME" \
    sh -c 'cargo build --release && mkdir -p /src/target/release && cp /build-target/release/fac_test /src/target/release/fac_test'

echo "=== 编译完成 ==="
ls -lh target/release/fac_test
file target/release/fac_test
echo "产物: target/release/fac_test"
