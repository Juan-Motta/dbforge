#!/usr/bin/env bash
# Renders every app icon the packages use from one 1024 px PNG, on macOS (sips, iconutil, python3):
#   packaging/icons/32x32.png, 128x128.png, 128x128@2x.png (256 px), icon.png (512 px)  Linux
#   packaging/icons/icon.icns                                                           macOS
#   packaging/icons/icon.ico (16 to 256 px, PNG frames)                                 Windows
#   docs/images/icon.png (256 px)                                                       README
# The source is kept as packaging/icons/app-icon.png; pass a new one to replace it.
# Usage: scripts/make-icons.sh [source.png]
set -euo pipefail

cd "$(dirname "$0")/.."
ICONS=packaging/icons
SOURCE="$ICONS/app-icon.png"
if [[ $# -gt 0 && "$1" != "$SOURCE" ]]; then
    cp "$1" "$SOURCE"
fi
read -r WIDTH HEIGHT < <(sips -g pixelWidth -g pixelHeight "$SOURCE" | awk '/pixel/ {printf "%s ", $2} END {print ""}')
if [[ "$WIDTH" != 1024 || "$HEIGHT" != 1024 ]]; then
    echo "$SOURCE must be 1024x1024 (it is ${WIDTH}x$HEIGHT)" >&2
    exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

render() { sips -z "$1" "$1" "$SOURCE" --out "$2" >/dev/null; }

render 32 "$ICONS/32x32.png"
render 128 "$ICONS/128x128.png"
render 256 "$ICONS/128x128@2x.png"
render 512 "$ICONS/icon.png"
render 256 docs/images/icon.png

# macOS: every size iconutil expects, 1x and 2x.
ICONSET="$WORK/icon.iconset"
mkdir "$ICONSET"
for size in 16 32 128 256 512; do
    render "$size" "$ICONSET/icon_${size}x${size}.png"
    render $((size * 2)) "$ICONSET/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$ICONSET" -o "$ICONS/icon.icns"

# Windows: an .ico holding PNG frames (Vista and later read them), smallest first.
FRAMES=()
for size in 16 24 32 48 64 128 256; do
    render "$size" "$WORK/$size.png"
    FRAMES+=("$WORK/$size.png")
done
python3 -I - "$ICONS/icon.ico" "${FRAMES[@]}" <<'PY'
import struct, sys

out, frames = sys.argv[1], sys.argv[2:]
images = [open(path, "rb").read() for path in frames]
header = struct.pack("<HHH", 0, 1, len(images))
offset = 6 + 16 * len(images)
entries, data = b"", b""
for image in images:
    width, height = struct.unpack(">II", image[16:24])
    entries += struct.pack(
        "<BBBBHHII", width % 256, height % 256, 0, 0, 1, 32, len(image), offset + len(data)
    )
    data += image
with open(out, "wb") as file:
    file.write(header + entries + data)
PY

echo "Rendered the icons from $SOURCE"
