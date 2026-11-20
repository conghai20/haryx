#!/bin/zsh
# Build a distributable HaryX.dmg with the haryx CLI bundled in the app.
#   scripts/package-dmg.sh             → apps/macos/build/HaryX-<version>.dmg (ad-hoc signed)
# Release signing (optional, both read from the environment):
#   SIGN_IDENTITY="Developer ID Application: Name (TEAMID)"   hardened-runtime signature
#   NOTARY_PROFILE=haryx    notarize + staple; create once with `xcrun notarytool store-credentials`
set -euo pipefail

ROOT=${0:A:h:h}            # apps/macos
REPO=${ROOT:h:h}           # repository root
BUILD="$ROOT/build"
APP="$BUILD/HaryX.app"
VERSION=$(sed -n 's/^VERSION="\(.*\)"/\1/p' "$ROOT/scripts/build.sh")
DMG="$BUILD/HaryX-$VERSION.dmg"

# 1. The CLI, built for this Mac's architecture like the app.
(cd "$REPO/cli" && cargo build --release -p haryx)
CLI="$REPO/cli/target/release/haryx"

# 2. The app, then the CLI inside it (Contents/Resources/bin/haryx, the app's last-resort lookup).
"$ROOT/scripts/build.sh"
mkdir -p "$APP/Contents/Resources/bin"
cp "$CLI" "$APP/Contents/Resources/bin/haryx"

# 3. Sign inside-out: the nested executable first, then the bundle.
if [[ -n "${SIGN_IDENTITY:-}" ]]; then
  SIGN=(codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY")
else
  echo "note: SIGN_IDENTITY unset; ad-hoc signing (Gatekeeper will warn on other Macs)" >&2
  SIGN=(codesign --force --sign - --timestamp=none)
fi
"${SIGN[@]}" "$APP/Contents/Resources/bin/haryx"
"${SIGN[@]}" "$APP"
codesign --verify --strict --deep "$APP"

# 4. The disk image: the app plus an Applications link to drag onto.
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create -quiet -volname "HaryX" -srcfolder "$STAGE" -fs HFS+ -format UDZO "$DMG"
[[ -n "${SIGN_IDENTITY:-}" ]] && codesign --force --timestamp --sign "$SIGN_IDENTITY" "$DMG"

# 5. Notarize when credentials are configured.
if [[ -n "${NOTARY_PROFILE:-}" ]]; then
  [[ -n "${SIGN_IDENTITY:-}" ]] || { echo "error: notarization needs SIGN_IDENTITY" >&2; exit 1; }
  xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple "$DMG"
fi

shasum -a 256 "$DMG" > "$DMG.sha256"
echo "Built $DMG"
