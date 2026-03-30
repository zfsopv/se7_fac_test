#!/bin/bash
set -e

IMAGE_NAME="fac-test-builder-win"
CONTAINER_CARGO_REGISTRY="fac-test-cargo-registry-win"

echo "=== 构建 Docker 编译镜像 (首次较慢, 后续秒级缓存) ==="
docker build -t "$IMAGE_NAME" -f - . <<'DOCKERFILE'
FROM ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y \
    curl build-essential pkg-config \
    gcc-mingw-w64-i686 \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

RUN rustup target add i686-pc-windows-gnu

RUN mkdir -p /root/.cargo && echo '\
[target.i686-pc-windows-gnu]\n\
linker = "i686-w64-mingw32-gcc"\n' > /root/.cargo/config.toml
DOCKERFILE

echo "=== 开始编译 Windows 32位版本 ==="
docker run --rm \
    -v "$PWD":/src \
    -v "$CONTAINER_CARGO_REGISTRY":/root/.cargo/registry \
    -w /src \
    "$IMAGE_NAME" \
    cargo build --release --target i686-pc-windows-gnu

echo "=== 编译完成 ==="
ls -lh target/i686-pc-windows-gnu/release/fac_test.exe
file target/i686-pc-windows-gnu/release/fac_test.exe
echo "产物: target/i686-pc-windows-gnu/release/fac_test.exe"
