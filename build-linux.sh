#!/bin/bash
set -e

IMAGE_NAME="fac-test-builder-alpine"
CONTAINER_CARGO_REGISTRY="fac-test-cargo-registry-alpine"
CONTAINER_TARGET="fac-test-alpine-target"

echo "=== 构建 Docker 编译镜像 (Alpine musl, 首次较慢, 后续秒级缓存) ==="
docker build -t "$IMAGE_NAME" -f - . <<'DOCKERFILE'
FROM alpine:3.23.3

RUN apk add --no-cache curl build-base perl linux-headers

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"
DOCKERFILE

echo "=== 开始编译 (Alpine musl 静态链接) ==="
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
echo "产物: target/release/fac_test (musl 静态链接, 兼容所有 Linux 发行版)"
