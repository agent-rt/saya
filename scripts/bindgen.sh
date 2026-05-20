#!/usr/bin/env bash
# Regenerate Swift bindings for the SwiftUI shell.
#
# Outputs into ./bindings/:
#   - saya.swift           — Swift binding API
#   - sayaFFI.h            — C header
#   - sayaFFI.modulemap    — module map
#
# The SwiftUI Xcode project consumes these plus the staticlib produced by
# `cargo build --release --features embedding`.

set -euo pipefail
cd "$(dirname "$0")/.."

PROFILE="${PROFILE:-debug}"
FEATURES="${FEATURES:-embedding}"
OUT_DIR="${OUT_DIR:-bindings}"

cargo_args=(-p saya-ffi --features "$FEATURES,uniffi-cli")
if [[ "$PROFILE" == "release" ]]; then
    cargo_args+=(--release)
fi

echo "==> building saya-ffi (profile=$PROFILE, features=$FEATURES)"
cargo build "${cargo_args[@]}"

echo "==> building uniffi-bindgen"
cargo build "${cargo_args[@]}" --bin uniffi-bindgen

mkdir -p "$OUT_DIR"
LIB="target/$PROFILE/libsaya_ffi.dylib"

echo "==> generating Swift bindings from $LIB"
"./target/$PROFILE/uniffi-bindgen" generate \
    --library "$LIB" \
    --language swift \
    --out-dir "$OUT_DIR"

echo "==> done. files in $OUT_DIR:"
ls -lh "$OUT_DIR"
