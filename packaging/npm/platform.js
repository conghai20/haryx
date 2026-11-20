// Release asset for this machine: haryx-v<version>-<rust target>.tar.gz on GitHub Releases.
"use strict";
const path = require("node:path");
const { version } = require("./package.json");

const TARGETS = {
  "darwin-arm64": "aarch64-apple-darwin",
  "darwin-x64": "x86_64-apple-darwin",
  "linux-arm64": "aarch64-unknown-linux-musl",
  "linux-x64": "x86_64-unknown-linux-musl",
};

function target(platform = process.platform, arch = process.arch) {
  const t = TARGETS[`${platform}-${arch}`];
  if (!t) throw new Error(`haryx has no prebuilt binary for ${platform}-${arch}; try: cargo install --locked haryx`);
  return t;
}

const RELEASES = process.env.HARYX_RELEASES_URL || "https://github.com/conghai20/haryx/releases/download";

function assetUrl(t = target()) {
  return `${RELEASES}/v${version}/haryx-v${version}-${t}.tar.gz`;
}

const binaryPath = path.join(__dirname, "vendor", "haryx");

module.exports = { TARGETS, target, assetUrl, binaryPath, version };
