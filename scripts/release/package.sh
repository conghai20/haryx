#!/bin/bash
# Package a built haryx binary as a release asset:
#   scripts/release/package.sh <rust-target> [binary] [out-dir]
# → <out-dir>/haryx-v<version>-<target>.tar.gz (haryx, LICENSE, README.md) and a .sha256 beside it.
# Default binary: cli/target/<target>/release/haryx; default out-dir: dist/.
set -euo pipefail

REPO=$(cd "$(dirname "$0")/../.." && pwd)
TARGET=${1:?usage: package.sh <rust-target> [binary] [out-dir]}
BIN=${2:-$REPO/cli/target/$TARGET/release/haryx}
OUT=${3:-$REPO/dist}
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$REPO/cli/Cargo.toml" | head -1)
NAME="haryx-v$VERSION-$TARGET"

[[ -x "$BIN" ]] || { echo "error: no executable at $BIN" >&2; exit 1; }
mkdir -p "$OUT"
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
cp "$BIN" "$STAGE/haryx"
cp "$REPO/LICENSE" "$STAGE/LICENSE"
cp "$REPO/cli/README.md" "$STAGE/README.md"
tar -czf "$OUT/$NAME.tar.gz" -C "$STAGE" haryx LICENSE README.md
(cd "$OUT" && shasum -a 256 "$NAME.tar.gz" > "$NAME.tar.gz.sha256")
echo "$OUT/$NAME.tar.gz"
