#!/bin/bash
# Package a built haryx binary as a release asset:
#   scripts/release/package.sh <rust-target> [binary] [out-dir]
# → <out-dir>/haryx-v<version>-<target>.tar.gz (haryx, LICENSE, README.md) and a .sha256 beside it.
# Windows targets carry haryx.exe and, when built, haryxw.exe (the windowless wake runner).
# Default binary: cli/target/<target>/release/haryx[.exe]; default out-dir: dist/.
set -euo pipefail

REPO=$(cd "$(dirname "$0")/../.." && pwd)
TARGET=${1:?usage: package.sh <rust-target> [binary] [out-dir]}
EXE=""
[[ "$TARGET" == *windows* ]] && EXE=".exe"
BIN=${2:-$REPO/cli/target/$TARGET/release/haryx$EXE}
OUT=${3:-$REPO/dist}
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$REPO/cli/Cargo.toml" | head -1)
NAME="haryx-v$VERSION-$TARGET"

[[ -f "$BIN" ]] || { echo "error: no executable at $BIN" >&2; exit 1; }
mkdir -p "$OUT"
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
FILES=("haryx$EXE")
cp "$BIN" "$STAGE/haryx$EXE"
WINDOWLESS="$(dirname "$BIN")/haryxw$EXE"
if [[ -n "$EXE" && -f "$WINDOWLESS" ]]; then
  cp "$WINDOWLESS" "$STAGE/haryxw$EXE"
  FILES+=("haryxw$EXE")
fi
cp "$REPO/LICENSE" "$STAGE/LICENSE"
cp "$REPO/cli/README.md" "$STAGE/README.md"
tar -czf "$OUT/$NAME.tar.gz" -C "$STAGE" "${FILES[@]}" LICENSE README.md
sha256() { if command -v shasum >/dev/null; then shasum -a 256 "$@"; else sha256sum "$@"; fi; }
(cd "$OUT" && sha256 "$NAME.tar.gz" > "$NAME.tar.gz.sha256")
echo "$OUT/$NAME.tar.gz"
