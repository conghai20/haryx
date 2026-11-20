#!/bin/zsh
# Export PNG and ICO files from the brand SVGs: app icons, favicons and preview renders.
# Requires the Xcode toolchain (swiftc) and Python 3 with Pillow (for .ico).
set -euo pipefail
ROOT=${0:A:h:h:h}
BRAND="$ROOT/assets/brand"
PNG="$BRAND/png"
BIN="$ROOT/.build/brand-render"
mkdir -p "$ROOT/.build" "$PNG"
[[ "$BIN" -nt "$ROOT/scripts/brand/render.swift" ]] || swiftc -O "$ROOT/scripts/brand/render.swift" -o "$BIN"

# App icons.
"$BIN" "$BRAND/haryx-app-icon.svg" "$PNG/haryx-app-icon-1024.png" 1024
for size in 16 32 48 64 128 256; do
  "$BIN" "$BRAND/haryx-app-icon-windows.svg" "$PNG/haryx-app-icon-windows-$size.png" $size
done
# Favicons.
for size in 16 32 48 180 512; do
  "$BIN" "$BRAND/favicon.svg" "$PNG/favicon-$size.png" $size
done
# Symbol previews.
"$BIN" "$BRAND/haryx-mark.svg" "$PNG/haryx-mark-512.png" 512 101827
"$BIN" "$BRAND/haryx-mark-light.svg" "$PNG/haryx-mark-light-512.png" 512 F1F5F9
# Lockup previews at 128 px high (width from each SVG's viewBox).
for name bg in haryx-lockup 101827 haryx-lockup-light F1F5F9; do
  vb_w=$(sed -E 's/.*viewBox="0 0 ([0-9.]+) 64".*/\1/' "$BRAND/$name.svg" | head -1)
  width=$(printf "%.0f" $(( vb_w * 2 + 0.5 )))
  "$BIN" "$BRAND/$name.svg" "$PNG/$name.png" "${width}x128" $bg
done

# Windows .ico and favicon.ico (multi-size).
python3 - "$PNG" "$BRAND" <<'PY'
import sys
from pathlib import Path
from PIL import Image
png, brand = Path(sys.argv[1]), Path(sys.argv[2])
win = [Image.open(png / f"haryx-app-icon-windows-{s}.png") for s in (16, 32, 48, 64, 128, 256)]
win[-1].save(brand / "haryx.ico", sizes=[im.size for im in win], append_images=win[:-1])
fav = [Image.open(png / f"favicon-{s}.png") for s in (16, 32, 48)]
fav[-1].save(brand / "favicon.ico", sizes=[im.size for im in fav], append_images=fav[:-1])
PY
echo "Exported PNG and ICO files to $BRAND"
