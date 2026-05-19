#!/usr/bin/env bash
# Installation script for kgr, the polyglot source dependency knowledge graph.
#
# Downloads a prebuilt binary from the latest GitHub Release for the host
# platform. Falls back to `cargo build` from source if downloads fail and cargo
# is available, or when KGR_FROM_SOURCE=1.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/joeaguilar/kgr/main/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/joeaguilar/kgr/main/install.sh | bash -s -- --update
#   ./install.sh
#   ./install.sh --update
#
# Environment overrides:
#   KGR_VERSION       Pin a specific release tag (e.g. v0.2.0). Defaults to latest.
#   KGR_INSTALL_DIR   Install directory. Defaults to an existing kgr on PATH,
#                     then ~/.cargo/bin when it already exists in PATH, then
#                     ~/.local/bin.
#   KGR_FROM_SOURCE   If 1, skip prebuilt download and build with cargo.
#   KGR_REPO          GitHub repo slug. Defaults to joeaguilar/kgr.

set -euo pipefail

REPO="${KGR_REPO:-joeaguilar/kgr}"
VERSION="${KGR_VERSION:-}"
FROM_SOURCE="${KGR_FROM_SOURCE:-0}"
ACTION="install"

SCRIPT_DIR=""
if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ]; then
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fi

if [ -t 1 ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[1;33m'
    BLUE='\033[0;34m'
    NC='\033[0m'
else
    RED=''; GREEN=''; YELLOW=''; BLUE=''; NC=''
fi

info()    { printf "${BLUE}i${NC} %s\n" "$1"; }
success() { printf "${GREEN}+${NC} %s\n" "$1"; }
warning() { printf "${YELLOW}!${NC} %s\n" "$1" >&2; }
error()   { printf "${RED}x${NC} %s\n" "$1" >&2; }

usage() {
    cat <<EOF
Usage:
  ./install.sh [--update]
  curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | bash -s -- [--update]

Options:
  --update    Update an existing kgr install if found on PATH; otherwise install it.
  -h, --help  Show this help text.

Environment:
  KGR_VERSION       Pin a specific release tag (defaults to latest).
  KGR_INSTALL_DIR   Override the install directory.
  KGR_FROM_SOURCE   Set to 1 to build from source in this repo.
  KGR_REPO          Override the GitHub repo slug.
EOF
}

parse_args() {
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --update|update)
                ACTION="update"
                ;;
            --install|install)
                ACTION="install"
                ;;
            -h|--help)
                usage
                exit 0
                ;;
            *)
                error "Unknown argument: $1"
                usage >&2
                exit 1
                ;;
        esac
        shift
    done
}

detect_target() {
    local uname_s uname_m
    uname_s=$(uname -s)
    uname_m=$(uname -m)

    case "$uname_s" in
        Darwin)
            case "$uname_m" in
                arm64|aarch64) echo "aarch64-apple-darwin" ;;
                x86_64)        echo "x86_64-apple-darwin" ;;
                *) error "Unsupported macOS architecture: $uname_m"; return 1 ;;
            esac
            ;;
        Linux)
            case "$uname_m" in
                x86_64)        echo "x86_64-unknown-linux-musl" ;;
                aarch64|arm64) echo "aarch64-unknown-linux-gnu" ;;
                *) error "Unsupported Linux architecture: $uname_m"; return 1 ;;
            esac
            ;;
        MINGW*|MSYS*|CYGWIN*)
            error "Detected Windows shell. Please use install.ps1 instead:"
            error "  iwr -useb https://raw.githubusercontent.com/${REPO}/main/install.ps1 | iex"
            return 1
            ;;
        *)
            error "Unsupported OS: $uname_s"; return 1 ;;
    esac
}

http_get() {
    local url="$1" out="$2"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$url" -o "$out"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$out" "$url"
    else
        error "Need curl or wget to download releases."
        return 1
    fi
}

resolve_version() {
    if [ -n "$VERSION" ]; then
        echo "$VERSION"
        return 0
    fi

    local url resolved
    url="https://github.com/${REPO}/releases/latest"
    if command -v curl >/dev/null 2>&1; then
        resolved=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$url")
    elif command -v wget >/dev/null 2>&1; then
        resolved=$(wget --max-redirect=5 -S --spider "$url" 2>&1 | awk '/^  Location: /{loc=$2} END{print loc}')
    else
        error "Need curl or wget to query the latest release."
        return 1
    fi

    echo "${resolved##*/}"
}

verify_checksum() {
    local file="$1" sumfile="$2" expected actual
    expected=$(awk '{print $1}' "$sumfile" | head -1)
    if command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "$file" | awk '{print $1}')
    elif command -v shasum >/dev/null 2>&1; then
        actual=$(shasum -a 256 "$file" | awk '{print $1}')
    else
        warning "No sha256sum/shasum found; skipping checksum verification."
        return 0
    fi

    if [ "$expected" != "$actual" ]; then
        error "Checksum mismatch: expected $expected, got $actual"
        return 1
    fi
    success "Checksum verified."
}

choose_install_dir() {
    if [ -n "${KGR_INSTALL_DIR:-}" ]; then
        echo "${KGR_INSTALL_DIR/#\~/$HOME}"
        return
    fi

    local existing
    existing=$(command -v kgr 2>/dev/null || true)
    if [ -n "$existing" ] && [ -f "$existing" ]; then
        dirname "$existing"
        return
    fi

    case ":$PATH:" in
        *":$HOME/.cargo/bin:"*)
            if [ -d "$HOME/.cargo/bin" ]; then
                echo "$HOME/.cargo/bin"
                return
            fi
            ;;
    esac

    echo "$HOME/.local/bin"
}

is_in_path() {
    case ":$PATH:" in
        *":$1:"*) return 0 ;;
        *)        return 1 ;;
    esac
}

existing_kgr_binary() {
    local install_dir="$1"
    if [ -x "$install_dir/kgr" ]; then
        echo "$install_dir/kgr"
    fi
}

print_existing_version() {
    local install_dir="$1" existing version
    existing=$(existing_kgr_binary "$install_dir")
    if [ -z "$existing" ]; then
        return
    fi

    version=$("$existing" --version 2>/dev/null || true)
    if [ -n "$version" ]; then
        info "Current install: $version ($existing)"
    else
        info "Current install: $existing"
    fi
}

install_binary() {
    local src="$1" install_dir="$2" existing_before="$3" need_sudo=0

    if ! mkdir -p "$install_dir" 2>/dev/null; then
        need_sudo=1
    elif [ ! -w "$install_dir" ]; then
        need_sudo=1
    fi

    if [ -n "$existing_before" ]; then
        info "Updating $install_dir/kgr"
    else
        info "Installing to $install_dir"
    fi

    if [ "$need_sudo" = 1 ]; then
        sudo mkdir -p "$install_dir"
        sudo install -m 0755 "$src" "$install_dir/kgr"
    else
        install -m 0755 "$src" "$install_dir/kgr"
    fi

    if [ -n "$existing_before" ]; then
        success "Updated $install_dir/kgr"
    else
        success "Installed $install_dir/kgr"
    fi

    if ! is_in_path "$install_dir"; then
        warning "$install_dir is not in your PATH."
        echo "  Add it with:"
        echo "    echo 'export PATH=\"$install_dir:\$PATH\"' >> ~/.bashrc   # or ~/.zshrc"
    fi

    "$install_dir/kgr" --version 2>/dev/null || true
}

install_from_release() {
    local target tag asset_base archive_url checksum_url tmpdir install_dir existing_before
    target=$(detect_target) || return 1
    info "Detected target: $target"

    tag=$(resolve_version)
    if [ -z "$tag" ] || [ "$tag" = "releases" ]; then
        error "Could not resolve latest release tag."
        return 1
    fi
    info "Release: $tag"

    asset_base="kgr-${tag}-${target}"
    archive_url="https://github.com/${REPO}/releases/download/${tag}/${asset_base}.tar.gz"
    checksum_url="${archive_url}.sha256"

    tmpdir=$(mktemp -d)
    trap 'rm -rf "$tmpdir"' RETURN

    info "Downloading ${asset_base}.tar.gz"
    if ! http_get "$archive_url" "$tmpdir/${asset_base}.tar.gz"; then
        error "Download failed: $archive_url"
        return 1
    fi
    if ! http_get "$checksum_url" "$tmpdir/${asset_base}.tar.gz.sha256"; then
        warning "Checksum file not found at $checksum_url"
    else
        verify_checksum "$tmpdir/${asset_base}.tar.gz" "$tmpdir/${asset_base}.tar.gz.sha256" || return 1
    fi

    info "Extracting..."
    tar -xzf "$tmpdir/${asset_base}.tar.gz" -C "$tmpdir"
    if [ ! -f "$tmpdir/${asset_base}/kgr" ]; then
        error "Extracted archive is missing the kgr binary."
        return 1
    fi

    install_dir=$(choose_install_dir)
    existing_before=$(existing_kgr_binary "$install_dir")
    print_existing_version "$install_dir"
    install_binary "$tmpdir/${asset_base}/kgr" "$install_dir" "$existing_before"
}

find_source_root() {
    local candidate
    for candidate in "$SCRIPT_DIR" "$PWD"; do
        if [ -n "$candidate" ] && [ -f "$candidate/Cargo.toml" ] && [ -d "$candidate/crates/kgr" ]; then
            echo "$candidate"
            return 0
        fi
    done

    error "Source build requires running this script from inside a cloned kgr repo."
    error "Try: git clone https://github.com/${REPO} && cd kgr && KGR_FROM_SOURCE=1 ./install.sh"
    return 1
}

install_from_source() {
    if ! command -v cargo >/dev/null 2>&1; then
        error "cargo is not installed; cannot build from source."
        error "Install Rust from https://rustup.rs/ and re-run, or use a release binary."
        return 1
    fi

    local source_root install_dir existing_before
    source_root=$(find_source_root) || return 1
    info "Building from source with cargo..."
    cargo build --release -p kgr --manifest-path "$source_root/Cargo.toml"

    install_dir=$(choose_install_dir)
    existing_before=$(existing_kgr_binary "$install_dir")
    print_existing_version "$install_dir"
    install_binary "$source_root/target/release/kgr" "$install_dir" "$existing_before"
}

main() {
    parse_args "$@"

    echo
    if [ "$ACTION" = "update" ]; then
        info "Updating kgr, the polyglot source dependency knowledge graph"
    else
        info "Installing kgr, the polyglot source dependency knowledge graph"
    fi
    echo

    if [ "$FROM_SOURCE" = "1" ]; then
        install_from_source
    elif install_from_release; then
        :
    else
        warning "Prebuilt install failed; falling back to cargo build."
        install_from_source
    fi

    echo
    success "Done."
    echo
    info "Quick start:"
    echo "  kgr                         # dependency tree of the current directory"
    echo "  kgr check --format json .   # CI/agent-friendly architecture check"
    echo "  kgr agent-info              # machine-readable guide for agents"
    echo
}

main "$@"
