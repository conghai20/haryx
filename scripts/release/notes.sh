#!/bin/bash
# Render a release's notes from CHANGELOG.md:
#   scripts/release/notes.sh <version> > notes.md      (version without the leading v)
# Prints the "## [<version>]" section (Breaking / Added / Changed / Fixed / Security ...), then the
# install lines and the compare link. Fails when the section is missing or empty, so every tag
# needs its changelog entry.
set -euo pipefail

REPO=$(cd "$(dirname "$0")/../.." && pwd)
VERSION=${1:?usage: notes.sh <version>}
VERSION=${VERSION#v}
CHANGELOG="$REPO/CHANGELOG.md"

# The lines after "## [<version>]" up to the next "## [" heading or the link references.
BODY=$(awk -v ver="$VERSION" '
  index($0, "## [" ver "]") == 1 { on = 1; next }
  on && (/^## \[/ || /^\[[^]]+\]: /) { exit }
  on { print }
' "$CHANGELOG" | sed -e '/./,$!d')   # drop leading blank lines

if [[ -z "${BODY//[[:space:]]/}" ]]; then
  echo "error: CHANGELOG.md has no entries under ## [$VERSION]" >&2
  exit 1
fi

PREVIOUS=$(grep -oE '^## \[[0-9][^]]*\]' "$CHANGELOG" | sed 's/^## \[\(.*\)\]/\1/' \
  | awk -v ver="$VERSION" 'found { print; exit } $0 == ver { found = 1 }')

printf '%s\n' "$BODY"
cat <<NOTES

### Install

- **macOS app (includes the CLI):** \`HaryX-<version>.dmg\` below; on first launch it offers to put
  \`haryx\` on your PATH.
- **CLI only:** the \`haryx-v$VERSION-<target>.tar.gz\` for your platform below, or
  \`cargo install --locked --git https://github.com/conghai20/haryx --tag v$VERSION haryx\`.
NOTES
if [[ -n "$PREVIOUS" ]]; then
  printf '\n**Full changelog:** https://github.com/conghai20/haryx/compare/v%s...v%s\n' "$PREVIOUS" "$VERSION"
fi
