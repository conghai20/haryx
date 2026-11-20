#!/usr/bin/env node
// Shim: run the native haryx binary, installing it first if postinstall was skipped
// (`npm install --ignore-scripts`). Exit code and signals pass straight through.
"use strict";
const fs = require("node:fs");
const { spawnSync } = require("node:child_process");
const { binaryPath } = require("../platform.js");
const { install } = require("../install.js");

async function main() {
  if (!fs.existsSync(binaryPath)) await install();
  const result = spawnSync(binaryPath, process.argv.slice(2), { stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.signal) process.kill(process.pid, result.signal);
  process.exit(result.status ?? 1);
}

main().catch((err) => {
  console.error(`haryx: ${err.message}`);
  process.exit(1);
});
