#!/usr/bin/env node
/*
 * `lystn` launcher for the npm package.
 *
 * Runs the prebuilt Rust binary downloaded by scripts/postinstall.js. If it's
 * missing (e.g. pnpm skipped postinstall), fetch it on the spot, then exec.
 */
"use strict";

const { spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");

const { download, binaryPath } = require(path.join(
  __dirname,
  "..",
  "scripts",
  "postinstall.js"
));

async function main() {
  const exe = binaryPath();
  if (!fs.existsSync(exe)) {
    const ok = await download();
    if (!ok || !fs.existsSync(exe)) {
      console.error(
        "[lystn] The lystn binary isn't installed and couldn't be downloaded.\n" +
          "[lystn] Check your network and run any `lystn` command again."
      );
      process.exit(1);
    }
  }
  const res = spawnSync(exe, process.argv.slice(2), { stdio: "inherit" });
  process.exit(res.status === null ? 1 : res.status);
}

main();
