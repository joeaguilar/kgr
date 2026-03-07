#!/usr/bin/env bash
set -euo pipefail

DEST="${HOME}/.cargo/bin/kgr"

# Resolve the repo root relative to this script, regardless of where it's called from.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "Building kgr (release)..."
cargo build --release -p kgr --manifest-path "${SCRIPT_DIR}/Cargo.toml"

install -m 755 "${SCRIPT_DIR}/target/release/kgr" "${DEST}"
echo "kgr installed to ${DEST}"
echo "Version: $("${DEST}" --version)"
