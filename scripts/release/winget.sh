#!/bin/bash
# Render the winget manifests for the HaryX Windows app from release checksums:
#   scripts/release/winget.sh [dist-dir] [out-dir]
# Needs HaryX-<version>-{x64,arm64}-setup.exe.sha256 in dist-dir (default dist/). Writes the three
# manifest files (version, installer, defaultLocale) to out-dir as winget-HaryX.HaryX*.yaml. Submit
# them to microsoft/winget-pkgs under manifests/h/HaryX/HaryX/<version>/ (without the winget- prefix).
set -euo pipefail

REPO=$(cd "$(dirname "$0")/../.." && pwd)
DIST=${1:-$REPO/dist}
OUT=${2:-$DIST}
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$REPO/cli/Cargo.toml" | head -1)
BASE="https://github.com/conghai20/haryx/releases/download/v$VERSION"
ID="HaryX.HaryX"
SCHEMA="1.9.0"

sum() {
  local file="$DIST/HaryX-$VERSION-$1-setup.exe.sha256"
  [[ -f "$file" ]] || { echo "error: missing $file" >&2; exit 1; }
  cut -d' ' -f1 < "$file" | tr '[:lower:]' '[:upper:]'
}

installer() {
  cat <<YAML
  - Architecture: $1
    InstallerUrl: $BASE/HaryX-$VERSION-$1-setup.exe
    InstallerSha256: $(sum "$1")
YAML
}

mkdir -p "$OUT"

cat > "$OUT/winget-$ID.yaml" <<YAML
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.version.$SCHEMA.schema.json
PackageIdentifier: $ID
PackageVersion: $VERSION
DefaultLocale: en-US
ManifestType: version
ManifestVersion: $SCHEMA
YAML

cat > "$OUT/winget-$ID.installer.yaml" <<YAML
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.installer.$SCHEMA.schema.json
PackageIdentifier: $ID
PackageVersion: $VERSION
InstallerType: nullsoft
Scope: user
InstallModes:
  - silent
  - silentWithProgress
UpgradeBehavior: install
Installers:
$(installer x64)
$(installer arm64)
ManifestType: installer
ManifestVersion: $SCHEMA
YAML

cat > "$OUT/winget-$ID.locale.en-US.yaml" <<YAML
# yaml-language-server: \$schema=https://aka.ms/winget-manifest.defaultLocale.$SCHEMA.schema.json
PackageIdentifier: $ID
PackageVersion: $VERSION
PackageLocale: en-US
Publisher: HaryX
PublisherUrl: https://github.com/conghai20/haryx
PackageName: HaryX
PackageUrl: https://github.com/conghai20/haryx
License: MIT
LicenseUrl: https://github.com/conghai20/haryx/blob/main/LICENSE
ShortDescription: A control center for Claude accounts, usage and sessions, in the Windows tray.
Tags:
  - claude
  - claude-code
  - ai
  - developer-tools
ManifestType: defaultLocale
ManifestVersion: $SCHEMA
YAML

echo "$OUT/winget-$ID.yaml"
