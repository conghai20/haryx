// Release asset for this machine: haryx-v<version>-<rust target>.tar.gz on GitHub Releases.
"use strict";
const path = require("node:path");
const { version } = require("./package.json");

const TARGETS = {
  "darwin-arm64": "aarch64-apple-darwin",
  "darwin-x64": "x86_64-apple-darwin",
  "linux-arm64": "aarch64-unknown-linux-musl",
  "linux-x64": "x86_64-unknown-linux-musl",
  "win32-arm64": "aarch64-pc-windows-msvc",
  "win32-x64": "x86_64-pc-windows-msvc",
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

/// The program files in a release archive for `platform`: Windows also carries the windowless wake runner.
function binaryNames(platform = process.platform) {
  return platform === "win32" ? ["haryx.exe", "haryxw.exe"] : ["haryx"];
}

const vendor = path.join(__dirname, "vendor");
const binaryPath = path.join(vendor, binaryNames()[0]);

module.exports = { TARGETS, target, assetUrl, binaryNames, binaryPath, vendor, version };
