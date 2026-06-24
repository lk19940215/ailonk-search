#!/usr/bin/env node
"use strict";

const fs = require("fs");
const { spawn, execFileSync } = require("child_process");
const path = require("path");

const CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000;
const ext = process.platform === "win32" ? ".exe" : "";
const bin = path.join(__dirname, `ailonk-search${ext}`);
const installScript = path.join(__dirname, "install.js");
const checkFile = path.join(__dirname, ".last-update-check");

function shouldCheckForUpdate() {
  if (!fs.existsSync(bin)) return true;
  try {
    const ts = parseInt(fs.readFileSync(checkFile, "utf8").trim(), 10);
    return Date.now() - ts > CHECK_INTERVAL_MS;
  } catch {
    return true;
  }
}

if (shouldCheckForUpdate()) {
  try {
    execFileSync(process.execPath, [installScript], {
      stdio: "inherit",
      timeout: 30_000,
      env: process.env,
    });
    fs.writeFileSync(checkFile, String(Date.now()));
  } catch (err) {
    console.error(`Auto-update check failed: ${err.message}`);
    console.error("Continuing with existing binary...");
  }
}

if (!fs.existsSync(bin)) {
  console.error("ailonk-search binary not found.");
  console.error("Run: npm rebuild ailonk-search");
  console.error("Or:  node install.js");
  process.exit(1);
}

const child = spawn(bin, process.argv.slice(2), {
  stdio: "inherit",
  env: process.env,
  windowsHide: true,
});

child.on("error", (err) => {
  console.error(err.message);
  process.exit(1);
});

for (const sig of ["SIGINT", "SIGTERM"]) {
  process.on(sig, () => {
    if (child.exitCode !== null || child.killed) return;
    child.kill(process.platform === "win32" ? undefined : sig);
  });
}

const SIGNAL_EXIT = { SIGINT: 130, SIGTERM: 143 };

child.on("exit", (code, signal) => {
  if (signal) {
    process.exit(SIGNAL_EXIT[signal] || 1);
    return;
  }
  process.exit(code != null ? code : 1);
});
