#!/bin/zsh
# Build HaryX.app (release) with the brand icon, ad-hoc sign, and optionally install.
#   scripts/build.sh            → apps/macos/build/HaryX.app
#   scripts/build.sh --install  → also copy to ~/Applications and launch
set -euo pipefail

ROOT=${0:A:h:h}            # apps/macos
REPO=${ROOT:h:h}           # repository root
BRAND_ICON="$REPO/assets/brand/png/haryx-app-icon-1024.png"
BUILD="$ROOT/build"
APP="$BUILD/HaryX.app"
VERSION="0.1.1"
BUNDLE_ID="dev.haryx.app"

cd "$ROOT"
swift build -c release --product HaryX
BIN="$(swift build -c release --show-bin-path)/HaryX"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/HaryX"

# Icon: the HaryX brand icon (assets/brand, regenerate with scripts/brand/export.sh).
# Falls back to the legacy sprite icon drawn by the binary if the brand PNG is missing.
ICONSET="$BUILD/AppIcon.iconset"
rm -rf "$ICONSET" && mkdir -p "$ICONSET"
if [[ -f "$BRAND_ICON" ]]; then
  cp "$BRAND_ICON" "$BUILD/icon-1024.png"
else
  echo "warning: $BRAND_ICON missing; using legacy sprite icon" >&2
  "$BIN" --render-icon "$BUILD/icon-1024.png"
fi
for size in 16 32 128 256 512; do
  sips -z $size $size "$BUILD/icon-1024.png" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  sips -z $((size * 2)) $((size * 2)) "$BUILD/icon-1024.png" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"

# Brand accent (deep Electric Cyan, white text 5.4:1): sidebar selection and prominent buttons.
xcrun actool "$ROOT/Resources/Assets.xcassets" --compile "$APP/Contents/Resources" --platform macosx \
  --minimum-deployment-target 26.0 --accent-color AccentColor \
  --output-partial-info-plist "$BUILD/assets-info.plist" >/dev/null

# Space Grotesk (OFL) for display moments, registered through ATSApplicationFontsPath.
mkdir -p "$APP/Contents/Resources/Fonts"
cp "$ROOT/Resources/Fonts/SpaceGrotesk.ttf" "$ROOT/Resources/Fonts/OFL.txt" "$APP/Contents/Resources/Fonts/"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleName</key><string>HaryX</string>
  <key>CFBundleDisplayName</key><string>HaryX</string>
  <key>CFBundleExecutable</key><string>HaryX</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>26.0</string>
  <key>LSUIElement</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>ATSApplicationFontsPath</key><string>Fonts</string>
  <key>NSAccentColorName</key><string>AccentColor</string>
  <key>NSAppleEventsUsageDescription</key><string>HaryX opens iTerm2 windows to run Claude as the account you picked.</string>
</dict>
</plist>
PLIST

codesign --force --deep --sign - --timestamp=none "$APP"
echo "Built $APP"

if [[ "${1:-}" == "--install" ]]; then
  DEST="$HOME/Applications/HaryX.app"
  mkdir -p "$HOME/Applications"
  pkill -x HaryX 2>/dev/null || true
  pkill -x cswapui 2>/dev/null || true  # the pre-rename build
  rm -rf "$DEST"
  cp -R "$APP" "$DEST"
  open "$DEST"
  echo "Installed and launched $DEST"
fi
