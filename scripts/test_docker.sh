#!/usr/bin/env bash
set -euo pipefail

# Runs truthdb-installer tests inside a Rust Debian container, similar to CI.
# Requirements:
#   - Docker Desktop / docker daemon running
# Usage:
#   ./scripts/test_docker.sh

IMAGE="rust:1.92-bookworm"
CONTAINER_NAME="truthdb-installer-test"

# Ensure we run from the installer repo root.
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cleanup() {
  docker rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true
}
trap cleanup EXIT

cleanup

docker create \
  --name "$CONTAINER_NAME" \
  -v "$REPO_ROOT":/work \
  -w /work \
  "$IMAGE" \
  sleep infinity >/dev/null

docker start "$CONTAINER_NAME" >/dev/null

echo "==> Running tests (glibc): x86_64-unknown-linux-gnu"
docker exec "$CONTAINER_NAME" cargo test --target x86_64-unknown-linux-gnu

echo "==> Installing musl tools"
docker exec "$CONTAINER_NAME" apt-get update >/dev/null
docker exec "$CONTAINER_NAME" apt-get install -y musl-tools >/dev/null

echo "==> Adding Rust musl target"
docker exec "$CONTAINER_NAME" rustup target add x86_64-unknown-linux-musl

echo "==> Running tests (musl): x86_64-unknown-linux-musl"
docker exec "$CONTAINER_NAME" cargo test --target x86_64-unknown-linux-musl

echo "==> OK"
