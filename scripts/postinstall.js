#!/usr/bin/env node
/*
 * Download the prebuilt Lystn (Rust) binary for this OS/arch.
 *
 * Runs at `npm install -g lystn-cli` (postinstall) AND lazily from bin/lystn.js
 * if the binary is missing (pnpm v10+ skips postinstall by default, so the
 * launcher must be able to self-fetch on first run).
 *
 * No Python, no compiler — just downloads the matching binary from the
 * lystn-cli GitHub Release that matches this package version.
 */
"use strict";

const fs = require("fs");
const path = require("path");
const https = require("https");

const PKG_ROOT = path.join(__dirname, "..");
const VERSION = require(path.join(PKG_ROOT, "package.json")).version;
const REPO = "burakayener/lystn-cli";
const BIN_DIR = path.join(PKG_ROOT, "binary");

// Map Node's platform/arch to the Rust target triple used in the release asset
// names (see .github/workflows/release.yml). `lystn-<triple>` (+ .exe on Windows).
function target() {
  const p = process.platform;
  const a = process.arch;
  if (p === "win32" && a === "x64") return { asset: "lystn-x86_64-pc-windows-msvc.exe", ext: ".exe" };
  // macOS: Apple Silicon only. Serve the arm64 binary for any darwin — an
  // M-series Mac running an x64 Node under Rosetta still runs arm64 natively.
  if (p === "darwin") return { asset: "lystn-aarch64-apple-darwin", ext: "" };
  if (p === "linux" && a === "x64") return { asset: "lystn-x86_64-unknown-linux-gnu", ext: "" };
  if (p === "linux" && a === "arm64") return { asset: "lystn-aarch64-unknown-linux-gnu", ext: "" };
  return null;
}

function binaryPath() {
  const ext = process.platform === "win32" ? ".exe" : "";
  return path.join(BIN_DIR, "lystn" + ext);
}

// GET with redirect following (GitHub release assets 302 to a CDN host).
function get(url, cb, redirects) {
  redirects = redirects || 0;
  if (redirects > 10) return cb(new Error("too many redirects"));
  https
    .get(url, { headers: { "User-Agent": "lystn-cli-installer" } }, (res) => {
      if ([301, 302, 303, 307, 308].includes(res.statusCode) && res.headers.location) {
        res.resume();
        return get(res.headers.location, cb, redirects + 1);
      }
      if (res.statusCode !== 200) {
        res.resume();
        return cb(new Error("HTTP " + res.statusCode + " for " + url));
      }
      cb(null, res);
    })
    .on("error", cb);
}

function download() {
  return new Promise((resolve) => {
    const t = target();
    if (!t) {
      console.error("[lystn] Unsupported platform: " + process.platform + "/" + process.arch);
      return resolve(false);
    }
    const url = `https://github.com/${REPO}/releases/download/v${VERSION}/${t.asset}`;
    const dest = binaryPath();
    const tmp = dest + ".download";
    try {
      fs.mkdirSync(BIN_DIR, { recursive: true });
    } catch (_) {}
    console.error("[lystn] Downloading the speech engine for your system ...");
    get(url, (err, res) => {
      if (err) {
        console.error("[lystn] Could not download the binary: " + err.message);
        return resolve(false);
      }
      const file = fs.createWriteStream(tmp);
      res.pipe(file);
      file.on("finish", () =>
        file.close(() => {
          try {
            fs.renameSync(tmp, dest);
            if (process.platform !== "win32") fs.chmodSync(dest, 0o755);
            resolve(true);
          } catch (e) {
            console.error("[lystn] Could not install the binary: " + e.message);
            resolve(false);
          }
        })
      );
      file.on("error", (e) => {
        console.error("[lystn] Download failed: " + e.message);
        try {
          fs.unlinkSync(tmp);
        } catch (_) {}
        resolve(false);
      });
    });
  });
}

module.exports = { download, binaryPath };

if (require.main === module) {
  // Postinstall: NEVER fail the npm install — the launcher self-fetches later.
  download().then((ok) => {
    if (ok) console.error("[lystn] Ready. Run: lystn install && lystn login");
    process.exit(0);
  });
}
