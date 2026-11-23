#!/bin/sh
# Offline tests for scripts/install.sh: a fake signed app in a local "release" served over file://.
#   scripts/test-install.sh
set -eu

REPO=$(cd "$(dirname "$0")/.." && pwd)
INSTALL="$REPO/scripts/install.sh"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
VERSION=9.9.9
FAILURES=0

fail() { echo "FAIL: $1" >&2; FAILURES=$((FAILURES + 1)); }
pass() { echo "ok: $1"; }

# A minimal app bundle with an executable and an ad-hoc signature, like the real ad-hoc DMG build.
make_app() { # dir marker
  mkdir -p "$1/HaryX.app/Contents/MacOS"
  cat > "$1/HaryX.app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>dev.haryx.install-test</string>
<key>CFBundleExecutable</key><string>HaryX</string>
<key>CFBundleShortVersionString</key><string>$2</string>
</dict></plist>
EOF
  cp /usr/bin/true "$1/HaryX.app/Contents/MacOS/HaryX"
  codesign --force --sign - "$1/HaryX.app" 2>/dev/null
}

# Lay out a release like GitHub's: <base>/download/v<version>/HaryX-<version>.dmg(.sha256).
make_release() { # base
  stage="$WORK/stage"
  rm -rf "$stage" && mkdir -p "$stage"
  make_app "$stage" "$VERSION"
  dir="$1/download/v$VERSION"
  mkdir -p "$dir"
  hdiutil create -quiet -volname HaryX -srcfolder "$stage" -fs HFS+ -format UDZO "$dir/HaryX-$VERSION.dmg"
  # The real checksum file names the DMG by its absolute build path; the installer must cope.
  (cd "$dir" && shasum -a 256 "$PWD/HaryX-$VERSION.dmg" > "HaryX-$VERSION.dmg.sha256")
}

run_install() { # base install_dir
  HARYX_BASE_URL="file://$1" HARYX_VERSION="$VERSION" HARYX_INSTALL_DIR="$2" HARYX_NO_OPEN=1 \
    sh "$INSTALL" > "$WORK/out.log" 2>&1
}

version_of() { defaults read "$1/HaryX.app/Contents/Info.plist" CFBundleShortVersionString 2>/dev/null; }

# 1. Fresh install: app present, signature valid, no quarantine flag.
BASE="$WORK/release"
make_release "$BASE"
DEST="$WORK/apps1"
mkdir -p "$DEST"
if run_install "$BASE" "$DEST"; then
  [ "$(version_of "$DEST")" = "$VERSION" ] && pass "fresh install copies the app" || fail "fresh install: wrong or missing app"
  codesign --verify --strict "$DEST/HaryX.app" 2>/dev/null && pass "signature intact" || fail "signature broken"
  xattr -p com.apple.quarantine "$DEST/HaryX.app" >/dev/null 2>&1 && fail "quarantine flag set" || pass "no quarantine flag"
else
  fail "fresh install exited non-zero: $(cat "$WORK/out.log")"
fi

# 2. Upgrade: an older app is replaced.
DEST="$WORK/apps2"
mkdir -p "$DEST"
make_app "$DEST" 0.0.1
if run_install "$BASE" "$DEST"; then
  [ "$(version_of "$DEST")" = "$VERSION" ] && pass "upgrade replaces the old app" || fail "upgrade kept the old app"
else
  fail "upgrade exited non-zero: $(cat "$WORK/out.log")"
fi

# 3. Checksum mismatch: refuse, and leave the existing app untouched.
BAD="$WORK/bad"
make_release "$BAD"
echo "0000000000000000000000000000000000000000000000000000000000000000  x.dmg" \
  > "$BAD/download/v$VERSION/HaryX-$VERSION.dmg.sha256"
DEST="$WORK/apps3"
mkdir -p "$DEST"
make_app "$DEST" 0.0.1
if run_install "$BAD" "$DEST"; then
  fail "bad checksum was accepted"
else
  grep -q "checksum" "$WORK/out.log" && pass "bad checksum refused" || fail "bad checksum: unclear error: $(cat "$WORK/out.log")"
  [ "$(version_of "$DEST")" = "0.0.1" ] && pass "existing app kept on failure" || fail "existing app changed on failure"
fi

# 4. Missing release: clear error, nothing installed.
DEST="$WORK/apps4"
mkdir -p "$DEST"
if run_install "$WORK/nowhere" "$DEST"; then
  fail "missing release was accepted"
else
  [ ! -e "$DEST/HaryX.app" ] && pass "missing release installs nothing" || fail "missing release left an app"
fi

# 5. No disk image left mounted by any run.
hdiutil info | grep -q "$WORK" && fail "a disk image is still mounted" || pass "no disk image left mounted"

[ "$FAILURES" -eq 0 ] && echo "all install tests passed" || { echo "$FAILURES failure(s)" >&2; exit 1; }
