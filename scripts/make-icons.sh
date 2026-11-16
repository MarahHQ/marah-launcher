#!/usr/bin/env bash
# Makes the app icons from assets/logo-static.svg (the logo without its animation), for the packages: assets/marah-launcher.icns (macOS),
# assets/marah-launcher.ico (Windows) and assets/marah-launcher.png (Linux, 512 px). The results are kept in
# the repo, so a build needs no image tools; run this again after changing the logo.
# Needs rsvg-convert (librsvg); the .icns needs macOS's iconutil, or libicns's png2icns elsewhere.
set -euo pipefail
cd "$(dirname "$0")/.."

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
render() { rsvg-convert -w "$1" -h "$1" assets/logo-static.svg -o "$2"; }

render 512 assets/marah-launcher.png

# Windows: an .ico holding PNGs of each size.
for size in 16 24 32 48 64 128 256; do render "$size" "$work/ico-$size.png"; done
python3 - "$work" assets/marah-launcher.ico <<'PY'
import struct, sys
work, out = sys.argv[1], sys.argv[2]
sizes = [16, 24, 32, 48, 64, 128, 256]
images = [open(f"{work}/ico-{s}.png", "rb").read() for s in sizes]
offset = 6 + 16 * len(sizes)
entries, data = b"", b""
for size, png in zip(sizes, images):
    side = 0 if size == 256 else size  # 0 means 256 in an ICO entry
    entries += struct.pack("<BBBBHHII", side, side, 0, 0, 1, 32, len(png), offset + len(data))
    data += png
open(out, "wb").write(struct.pack("<HHH", 0, 1, len(sizes)) + entries + data)
PY

# macOS: an icon set, made into an .icns.
if command -v iconutil > /dev/null; then
    set_dir="$work/marah-launcher.iconset"
    mkdir -p "$set_dir"
    for size in 16 32 128 256 512; do
        render "$size" "$set_dir/icon_${size}x${size}.png"
        render "$((size * 2))" "$set_dir/icon_${size}x${size}@2x.png"
    done
    iconutil -c icns "$set_dir" -o assets/marah-launcher.icns
elif command -v png2icns > /dev/null; then
    # Elsewhere, libicns's png2icns: one PNG for each size it stores, up to 1024 px.
    for size in 16 32 48 128 256 512 1024; do render "$size" "$work/icns-$size.png"; done
    # It prints its JPEG 2000 library's deprecation warnings each time: shown only when it fails.
    if ! png2icns assets/marah-launcher.icns "$work"/icns-{16,32,48,128,256,512,1024}.png > "$work/png2icns.log" 2>&1; then
        cat "$work/png2icns.log" >&2
        exit 1
    fi
else
    echo "neither iconutil (macOS) nor png2icns (libicns) found: assets/marah-launcher.icns left as it is" >&2
fi
echo "made assets/marah-launcher.png, assets/marah-launcher.ico and assets/marah-launcher.icns"
