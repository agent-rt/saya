#!/usr/bin/env bash
# Render assets/icon.svg → AppIcon.icns at all macOS-required sizes.
# Drops the result at apps/saya-macos/Resources/AppIcon.icns.

set -euo pipefail
cd "$(dirname "$0")/.."

SRC="assets/icon.svg"
OUT_ICNS="apps/saya-macos/Resources/AppIcon.icns"
TMP="$(mktemp -d /tmp/saya-icon-XXXXXX)"
trap 'rm -rf "$TMP"' EXIT

if [ ! -f "$SRC" ]; then
    echo "missing $SRC"
    exit 1
fi

echo "==> rendering master at 1024×1024"
qlmanage -t -s 1024 -o "$TMP" "$SRC" >/dev/null 2>&1
MASTER="$TMP/$(basename "$SRC").png"

ICONSET="$TMP/AppIcon.iconset"
mkdir -p "$ICONSET"

# (size_at_1x, output_basename)
declare -a SIZES=(
    "16:icon_16x16"
    "32:icon_16x16@2x"
    "32:icon_32x32"
    "64:icon_32x32@2x"
    "128:icon_128x128"
    "256:icon_128x128@2x"
    "256:icon_256x256"
    "512:icon_256x256@2x"
    "512:icon_512x512"
    "1024:icon_512x512@2x"
)

for entry in "${SIZES[@]}"; do
    px="${entry%%:*}"
    name="${entry##*:}"
    sips -s format png -z "$px" "$px" "$MASTER" --out "$ICONSET/$name.png" >/dev/null
done

echo "==> compiling .icns"
mkdir -p "$(dirname "$OUT_ICNS")"
iconutil -c icns "$ICONSET" -o "$OUT_ICNS"

echo "==> done"
ls -lh "$OUT_ICNS"
