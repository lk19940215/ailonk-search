#!/usr/bin/env node
"use strict";

const { spawn } = require("child_process");
const path = require("path");

const ext = process.platform === "win32" ? ".exe" : "";
const bin = path.join(__dirname, `ailonk-search${ext}`);

const child = spawn(bin, process.argv.slice(2), {
  stdio: "inherit",
  env: process.env,
});

child.on("error", (err) => {
  if (err.code === "ENOENT") {
    console.error(
      "ailonk-search binary not found. Run: npm rebuild ailonk-search"
    );
  } else {
    console.error(err.message);
  }
  process.exit(1);
});

child.on("exit", (code) => process.exit(code ?? 1));
