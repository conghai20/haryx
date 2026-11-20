#!/bin/zsh
# Build cswapui.app (release), render its icon, ad-hoc sign, and optionally install.
#   scripts/build.sh            → build/cswapui.app
#   scripts/build.sh --install  → also copy to ~/Applications and launch
set -euo pipefail

ROOT=${0:A:h:h}
BUILD="$ROOT/build"
APP="$BUILD/cswapui.app"
VERSION="1.0.0"
BUNDLE_ID="dev.cswapui.app"

cd "$ROOT"
swift build -c release --product cswapui
BIN="$(swift build -c release --show-bin-path)/cswapui"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/cswapui"

# Icon: the binary draws it from the same sprite data it animates.
ICONSET="$BUILD/AppIcon.iconset"
rm -rf "$ICONSET" && mkdir -p "$ICONSET"
"$BIN" --render-icon "$BUILD/icon-1024.png"
for size in 16 32 128 256 512; do
  sips -z $size $size "$BUILD/icon-1024.png" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  sips -z $((size * 2)) $((size * 2)) "$BUILD/icon-1024.png" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleName</key><string>cswapui</string>
  <key>CFBundleDisplayName</key><string>cswapui</string>
  <key>CFBundleExecutable</key><string>cswapui</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>26.0</string>
  <key>LSUIElement</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSAppleEventsUsageDescription</key><string>cswapui opens iTerm2 windows to run Claude as the account you picked.</string>
</dict>
</plist>
PLIST

codesign --force --deep --sign - --timestamp=none "$APP"
echo "Built $APP"

if [[ "${1:-}" == "--install" ]]; then
  DEST="$HOME/Applications/cswapui.app"
  mkdir -p "$HOME/Applications"
  pkill -x cswapui 2>/dev/null || true
  rm -rf "$DEST"
  cp -R "$APP" "$DEST"
  open "$DEST"
  echo "Installed and launched $DEST"
fi
