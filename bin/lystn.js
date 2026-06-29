#!/usr/bin/env node
/*
 * `lystn` launcher for the npm package.
 *
 * The prebuilt Rust binaries for every supported platform are BUNDLED inside
 * this package (the `binary/` dir, populated at publish time by CI). We just
 * pick the one matching this machine and exec it.
 *
 * No postinstall script, no install-time download — so npm raises no
 * `allow-scripts` warning and there's nothing to fail on a restricted network.
 */
"use strict";

const { spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");

// Map Node's platform/arch to the bundled binary filename (Rust target triple).
function binName() {
  const p = process.platform;
  const a = process.arch;
  if (p === "win32" && a === "x64") return "lystn-x86_64-pc-windows-msvc.exe";
  // macOS: Apple Silicon binary. An x64 Node under Rosetta on an M-series Mac
  // still runs the arm64 binary natively.
  if (p === "darwin") return "lystn-aarch64-apple-darwin";
  if (p === "linux" && a === "x64") return "lystn-x86_64-unknown-linux-gnu";
  if (p === "linux" && a === "arm64") return "lystn-aarch64-unknown-linux-gnu";
  return null;
}

const name = binName();
if (!name) {
  console.error(
    "[lystn] Unsupported platform: " + process.platform + "/" + process.arch
  );
  process.exit(1);
}

const exe = path.join(__dirname, "..", "binary", name);
if (!fs.existsSync(exe)) {
  console.error(
    "[lystn] The bundled binary for this platform (" + name + ") is missing.\n" +
      "[lystn] Please reinstall: npm install -g lystn-cli"
  );
  process.exit(1);
}

// Belt-and-suspenders: npm preserves the executable bit from the tarball, but
// ensure it on first run too (ignored if we can't write to a global install).
if (process.platform !== "win32") {
  try {
    fs.accessSync(exe, fs.constants.X_OK);
  } catch (_) {
    try {
      fs.chmodSync(exe, 0o755);
    } catch (_) {
      /* not writable (root-owned global) — rely on the tarball's mode */
    }
  }
}

const res = spawnSync(exe, process.argv.slice(2), { stdio: "inherit" });
process.exit(res.status === null ? 1 : res.status);
