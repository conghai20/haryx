#!/bin/sh
# Install or update the HaryX macOS app (with the bundled haryx CLI) from GitHub releases:
#
#   curl -fsSL https://raw.githubusercontent.com/conghai20/haryx/main/scripts/install.sh | sh
#
# curl does not mark what it downloads as "from the internet", so macOS opens the app without the
# Privacy & Security approval a browser-downloaded DMG needs. The DMG is checked against the release's
# SHA-256 file before anything is installed.
#
# Environment (all optional):
#   HARYX_VERSION      install this version (e.g. 0.1.1) instead of the latest release
#   HARYX_INSTALL_DIR  where HaryX.app goes (default /Applications)
#   HARYX_NO_OPEN=1    do not launch the app after installing
#   HARYX_BASE_URL     releases base URL (default https://github.com/conghai20/haryx/releases)
set -eu

APP_NAME="HaryX.app"
DEFAULT_BASE_URL="https://github.com/conghai20/haryx/releases"
MIN_MACOS_MAJOR=26
QUIT_WAIT_SECONDS=10

say() { printf '%s\n' "$*"; }
die() { printf 'haryx install: %s\n' "$*" >&2; exit 1; }

check_platform() {
  [ "$(uname -s)" = "Darwin" ] || die "HaryX is a macOS app. For the CLI on other systems, see the README."
  [ "$(uname -m)" = "arm64" ] || die "the HaryX app is built for Apple silicon (arm64) only."
  major=$(sw_vers -productVersion | cut -d. -f1)
  [ "$major" -ge "$MIN_MACOS_MAJOR" ] || die "HaryX needs macOS $MIN_MACOS_MAJOR or later (this Mac: $(sw_vers -productVersion))."
}

# The latest version, from where GitHub's /releases/latest redirects (no API call, no rate limit).
resolve_version() { # base_url
  if [ -n "${HARYX_VERSION:-}" ]; then
    printf '%s' "${HARYX_VERSION#v}"
    return
  fi
  url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$1/latest") || die "cannot reach $1/latest"
  tag=${url##*/tag/}
  [ "$tag" != "$url" ] && [ -n "$tag" ] || die "no published release found at $1"
  printf '%s' "${tag#v}"
}

download() { # url dest
  curl -fsSL --retry 2 -o "$2" "$1" || die "download failed: $1"
}

# The checksum file's first field is the hash; its file name may be an absolute build path, so it is ignored.
verify_checksum() { # dmg sha_file
  expected=$(awk 'NR==1 {print $1}' "$2")
  actual=$(shasum -a 256 "$1" | awk '{print $1}')
  [ -n "$expected" ] && [ "$expected" = "$actual" ] \
    || die "checksum mismatch for $(basename "$1"); nothing was installed."
}

# Quit a running copy of the app at this exact path the normal way. Never force-kills.
quit_running_app() { # app_path
  pgrep -f "$1/Contents/MacOS/" >/dev/null 2>&1 || return 0
  bundle_id=$(defaults read "$1/Contents/Info.plist" CFBundleIdentifier 2>/dev/null) \
    || die "HaryX is running and its bundle id is unreadable; quit it and run this again."
  say "Quitting the running HaryX..."
  osascript -e "tell application id \"$bundle_id\" to quit" >/dev/null 2>&1 || true
  waited=0
  while pgrep -f "$1/Contents/MacOS/" >/dev/null 2>&1; do
    [ "$waited" -lt "$QUIT_WAIT_SECONDS" ] || die "HaryX did not quit; quit it and run this again."
    sleep 1
    waited=$((waited + 1))
  done
}

# Copy the new app next to the old one, then swap; the old app comes back if the swap fails.
install_app() { # source_app install_dir
  target="$2/$APP_NAME"
  staged="$2/.$APP_NAME.new.$$"
  backup="$2/.$APP_NAME.old.$$"
  ditto "$1" "$staged" || { rm -rf "$staged"; die "cannot write to $2 (try HARYX_INSTALL_DIR=~/Applications)."; }
  if [ -e "$target" ]; then
    mv "$target" "$backup" || { rm -rf "$staged"; die "cannot replace $target"; }
  fi
  if ! mv "$staged" "$target"; then
    [ -e "$backup" ] && mv "$backup" "$target"
    rm -rf "$staged"
    die "cannot move the new app into $2"
  fi
  rm -rf "$backup"
  codesign --verify --strict "$target" 2>/dev/null || die "the installed app's signature does not verify: $target"
}

main() {
  check_platform
  base=${HARYX_BASE_URL:-$DEFAULT_BASE_URL}
  install_dir=${HARYX_INSTALL_DIR:-/Applications}
  [ -d "$install_dir" ] || die "install folder does not exist: $install_dir"

  version=$(resolve_version "$base")
  dmg_name="HaryX-$version.dmg"
  work=$(mktemp -d)
  mount="$work/mnt"
  trap 'hdiutil detach -quiet "$mount" >/dev/null 2>&1 || true; rm -rf "$work"' EXIT
  trap 'exit 130' INT TERM

  say "Downloading HaryX $version..."
  download "$base/download/v$version/$dmg_name" "$work/$dmg_name"
  download "$base/download/v$version/$dmg_name.sha256" "$work/$dmg_name.sha256"
  verify_checksum "$work/$dmg_name" "$work/$dmg_name.sha256"

  mkdir -p "$mount"
  hdiutil attach -quiet -nobrowse -readonly -mountpoint "$mount" "$work/$dmg_name" \
    || die "cannot open the disk image $dmg_name"
  [ -d "$mount/$APP_NAME" ] || die "$dmg_name does not contain $APP_NAME"

  quit_running_app "$install_dir/$APP_NAME"
  install_app "$mount/$APP_NAME" "$install_dir"
  hdiutil detach -quiet "$mount" >/dev/null 2>&1 || true
  say "Installed HaryX $version in $install_dir/$APP_NAME"

  if [ "${HARYX_NO_OPEN:-}" != "1" ]; then
    open "$install_dir/$APP_NAME"
  fi
}

# Everything runs from here, so a partly downloaded script does nothing.
main "$@"
