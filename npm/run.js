#!/usr/bin/env node
"use strict";

const fs = require("fs");
const { spawn } = require("child_process");
const path = require("path");

const ext = process.platform === "win32" ? ".exe" : "";
const bin = path.join(__dirname, `ailonk-search${ext}`);

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
