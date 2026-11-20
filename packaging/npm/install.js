// postinstall: download this platform's release binary, verify its SHA-256, unpack to vendor/.
"use strict";
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const { assetUrl, binaryPath } = require("./platform.js");

async function fetchBuffer(url) {
  const res = await fetch(url, { redirect: "follow" });
  if (!res.ok) throw new Error(`GET ${url}: HTTP ${res.status}`);
  return Buffer.from(await res.arrayBuffer());
}

async function install() {
  if (fs.existsSync(binaryPath)) return;
  const url = assetUrl();
  const [archive, sums] = await Promise.all([fetchBuffer(url), fetchBuffer(`${url}.sha256`)]);
  const expected = sums.toString("utf8").trim().split(/\s+/)[0];
  const actual = crypto.createHash("sha256").update(archive).digest("hex");
  if (expected !== actual) throw new Error(`checksum mismatch for ${url}: expected ${expected}, got ${actual}`);
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "haryx-"));
  try {
    const file = path.join(tmp, "haryx.tar.gz");
    fs.writeFileSync(file, archive);
    execFileSync("tar", ["-xzf", file, "-C", tmp]);
    const unpacked = path.join(tmp, "haryx");
    fs.mkdirSync(path.dirname(binaryPath), { recursive: true });
    fs.copyFileSync(unpacked, binaryPath);
    fs.chmodSync(binaryPath, 0o755);
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
}

module.exports = { install };

if (require.main === module) {
  install().catch((err) => {
    console.error(`haryx: install failed: ${err.message}`);
    process.exit(1);
  });
}
