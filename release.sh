#!/bin/bash
set -euo pipefail

BINARY="chaos"
VERSION="${1:-$(grep '^version' crates/chaos-cli/Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')}"
RELEASE_DIR="target/release-artifacts"

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

info() { echo -e "${GREEN}[INFO]${NC} $*"; }
error() { echo -e "${RED}[ERROR]${NC} $*" >&2; exit 1; }

LINUX_TARGETS=(
    x86_64-unknown-linux-gnu
    x86_64-unknown-linux-musl
    aarch64-unknown-linux-gnu
    aarch64-unknown-linux-musl
)

MACOS_TARGETS=(
    x86_64-apple-darwin
    aarch64-apple-darwin
)

mkdir -p "$RELEASE_DIR"

# --- Linux targets: build inside Docker ---
build_linux() {
    local target="$1"

    info "Building ${target} (Docker)..."

    docker build \
        --build-arg TARGET="$target" \
        -f Dockerfile.release \
        -t "chaos-build-${target}" \
        .

    local container_id
    container_id=$(docker create "chaos-build-${target}")
    docker cp "${container_id}:/out/${BINARY}" "${RELEASE_DIR}/${BINARY}"
    docker rm "$container_id" > /dev/null

    tar -czf "${RELEASE_DIR}/${BINARY}-${VERSION}-${target}.tar.gz" \
        -C "$RELEASE_DIR" "$BINARY"
    rm -f "${RELEASE_DIR}/${BINARY}"

    info "Done: ${BINARY}-${VERSION}-${target}.tar.gz"
}

# --- macOS targets: build natively ---
build_macos() {
    local target="$1"

    info "Building ${target} (native)..."

    rustup target add "$target" 2>/dev/null || true
    cargo build --release --target "$target" -p chaos-cli

    tar -czf "${RELEASE_DIR}/${BINARY}-${VERSION}-${target}.tar.gz" \
        -C "target/${target}/release" "$BINARY"

    info "Done: ${BINARY}-${VERSION}-${target}.tar.gz"
}

# --- Build all ---
if command -v docker &>/dev/null && docker info &>/dev/null 2>&1; then
    for target in "${LINUX_TARGETS[@]}"; do
        build_linux "$target"
    done
else
    info "Docker not available, skipping Linux targets"
fi

if [[ "$(uname -s)" == "Darwin" ]]; then
    for target in "${MACOS_TARGETS[@]}"; do
        build_macos "$target"
    done
else
    info "Not on macOS, skipping Darwin targets"
fi

# --- List artifacts ---
echo ""
info "Artifacts:"
ls -lh "$RELEASE_DIR"/*.tar.gz

# --- Push release ---
echo ""
read -rp "Create GitHub release v${VERSION}? [y/N] " confirm
if [[ "$confirm" =~ ^[Yy]$ ]]; then
    gh release create "v${VERSION}" \
        --title "v${VERSION}" \
        --generate-notes \
        "${RELEASE_DIR}"/*.tar.gz
    info "Released v${VERSION}"
else
    info "Skipped. Run manually: gh release create v${VERSION} ${RELEASE_DIR}/*.tar.gz"
fi
