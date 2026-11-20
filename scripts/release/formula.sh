#!/bin/bash
# Render the Homebrew formula from release checksums:
#   scripts/release/formula.sh [dist-dir] > haryx.rb
# Needs haryx-v<version>-<target>.tar.gz.sha256 for the four macOS/Linux targets in dist-dir
# (default dist/). Publish the result to the tap repo (conghai20/homebrew-tap, Formula/haryx.rb).
set -euo pipefail

REPO=$(cd "$(dirname "$0")/../.." && pwd)
DIST=${1:-$REPO/dist}
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$REPO/cli/Cargo.toml" | head -1)
BASE="https://github.com/conghai20/haryx/releases/download/v$VERSION"

sum() {
  local file="$DIST/haryx-v$VERSION-$1.tar.gz.sha256"
  [[ -f "$file" ]] || { echo "error: missing $file" >&2; exit 1; }
  cut -d' ' -f1 < "$file"
}

asset() {
  printf '      url "%s/haryx-v%s-%s.tar.gz"\n      sha256 "%s"\n' "$BASE" "$VERSION" "$1" "$(sum "$1")"
}

cat <<RUBY
class Haryx < Formula
  desc "Unified control center for AI coding accounts, usage and sessions"
  homepage "https://github.com/conghai20/haryx"
  version "$VERSION"
  license "MIT"

  on_macos do
    on_arm do
$(asset aarch64-apple-darwin)
    end
    on_intel do
$(asset x86_64-apple-darwin)
    end
  end

  on_linux do
    on_arm do
$(asset aarch64-unknown-linux-musl)
    end
    on_intel do
$(asset x86_64-unknown-linux-musl)
    end
  end

  def install
    bin.install "haryx"
  end

  test do
    assert_match "haryx #{version}", shell_output("#{bin}/haryx --version")
  end
end
RUBY
