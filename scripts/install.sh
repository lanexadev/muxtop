#!/bin/sh
# install.sh — Install muxtop from GitHub Releases
#
# Usage:
#   curl -sSfL https://raw.githubusercontent.com/lanexadev/muxtop/main/scripts/install.sh | sh
#
# Environment variables:
#   MUXTOP_VERSION  — version to install (default: latest)
#   MUXTOP_DIR      — install directory (default: /usr/local/bin or ~/.local/bin)

set -eu

REPO="lanexadev/muxtop"
BINARY="muxtop"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

say() {
    printf '%s\n' "$*"
}

err() {
    say "error: $*" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || err "need '$1' (command not found)"
}

# ---------------------------------------------------------------------------
# Detect OS and architecture
# ---------------------------------------------------------------------------

detect_target() {
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)  os_part="unknown-linux-musl" ;;
        Darwin) os_part="apple-darwin" ;;
        *)      err "unsupported OS: $os" ;;
    esac

    case "$arch" in
        x86_64|amd64)   arch_part="x86_64" ;;
        aarch64|arm64)  arch_part="aarch64" ;;
        *)              err "unsupported architecture: $arch" ;;
    esac

    TARGET="${arch_part}-${os_part}"
}

# ---------------------------------------------------------------------------
# Resolve version (latest tag if not specified)
# ---------------------------------------------------------------------------

resolve_version() {
    if [ -n "${MUXTOP_VERSION:-}" ]; then
        VERSION="$MUXTOP_VERSION"
    else
        need curl
        VERSION="$(curl -sSfL "https://api.github.com/repos/${REPO}/releases/latest" \
            | grep '"tag_name"' \
            | head -1 \
            | sed 's/.*"tag_name": *"//;s/".*//')" \
            || err "failed to fetch latest release version"
        [ -n "$VERSION" ] || err "could not determine latest version"
    fi
}

# ---------------------------------------------------------------------------
# Download and verify
# ---------------------------------------------------------------------------

download_and_install() {
    need curl
    need tar

    archive="${BINARY}-${TARGET}.tar.gz"
    checksum_file="${archive}.sha256"
    base_url="https://github.com/${REPO}/releases/download/${VERSION}"

    tmpdir="$(mktemp -d)"
    trap 'rm -rf "$tmpdir"' EXIT

    say "Downloading ${BINARY} ${VERSION} for ${TARGET}..."
    curl -sSfL -o "${tmpdir}/${archive}" "${base_url}/${archive}" \
        || err "download failed — check that ${VERSION} has a release for ${TARGET}"

    curl -sSfL -o "${tmpdir}/${checksum_file}" "${base_url}/${checksum_file}" \
        || err "checksum download failed"

    # Verify SHA256
    say "Verifying checksum..."
    cd "$tmpdir"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum -c "$checksum_file" || err "checksum verification failed"
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 -c "$checksum_file" || err "checksum verification failed"
    else
        err "need 'sha256sum' or 'shasum' for checksum verification"
    fi

    # Extract
    tar xzf "$archive"

    # Determine install directory
    if [ -n "${MUXTOP_DIR:-}" ]; then
        install_dir="$MUXTOP_DIR"
    elif [ "$(id -u)" -eq 0 ]; then
        install_dir="/usr/local/bin"
    else
        install_dir="${HOME}/.local/bin"
    fi

    mkdir -p "$install_dir"

    # Install
    cp "${BINARY}" "${install_dir}/${BINARY}"
    chmod +x "${install_dir}/${BINARY}"

    say ""
    say "Installed ${BINARY} to ${install_dir}/${BINARY}"

    # Verify
    if "${install_dir}/${BINARY}" --version 2>/dev/null; then
        :
    else
        say ""
        say "Note: ${install_dir} may not be in your PATH."
        say "Add it with:  export PATH=\"${install_dir}:\$PATH\""
    fi
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

main() {
    say "muxtop installer"
    say ""

    detect_target
    resolve_version
    download_and_install

    say ""
    say "Done."
}

main
