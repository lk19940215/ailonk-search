#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");
const https = require("https");
const http = require("http");

const REPO = "lk19940215/ailonk-search";
const BIN_NAME = "ailonk-search";
const VERSION = require("./package.json").version;

function getPlatformTarget() {
  const arch = process.arch;
  const platform = process.platform;

  const map = {
    "darwin-arm64": "aarch64-apple-darwin",
    "darwin-x64": "x86_64-apple-darwin",
    "linux-x64": "x86_64-unknown-linux-gnu",
    "win32-x64": "x86_64-pc-windows-msvc",
  };

  const key = `${platform}-${arch}`;
  const target = map[key];
  if (!target) {
    console.error(`Unsupported platform: ${key}`);
    console.error(`Supported: ${Object.keys(map).join(", ")}`);
    process.exit(1);
  }
  return target;
}

function getBinaryName(target) {
  return target.includes("windows")
    ? `${BIN_NAME}-${target}.exe`
    : `${BIN_NAME}-${target}`;
}

function download(url) {
  return new Promise((resolve, reject) => {
    const client = url.startsWith("https") ? https : http;
    client
      .get(url, { headers: { "User-Agent": "ailonk-search-npm" } }, (res) => {
        if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
          return download(res.headers.location).then(resolve, reject);
        }
        if (res.statusCode !== 200) {
          return reject(new Error(`HTTP ${res.statusCode} for ${url}`));
        }
        const chunks = [];
        res.on("data", (c) => chunks.push(c));
        res.on("end", () => resolve(Buffer.concat(chunks)));
        res.on("error", reject);
      })
      .on("error", reject);
  });
}

async function main() {
  const target = getPlatformTarget();
  const assetName = getBinaryName(target);
  const tag = `v${VERSION}`;
  const url = `https://github.com/${REPO}/releases/download/${tag}/${assetName}`;

  const binDir = __dirname;
  const dest = path.join(
    binDir,
    process.platform === "win32" ? `${BIN_NAME}.exe` : BIN_NAME
  );

  const versionFile = path.join(binDir, ".installed-version");
  if (fs.existsSync(dest) && fs.existsSync(versionFile)) {
    const installed = fs.readFileSync(versionFile, "utf8").trim();
    if (installed === VERSION) {
      console.log(`${BIN_NAME} v${VERSION} already installed`);
      return;
    }
  }

  console.log(`Downloading ${BIN_NAME} ${tag} for ${target}...`);
  console.log(`  ${url}`);

  try {
    const data = await download(url);
    fs.writeFileSync(dest, data);
    if (process.platform !== "win32") {
      fs.chmodSync(dest, 0o755);
    }
    fs.writeFileSync(versionFile, VERSION);
    console.log(`Installed ${BIN_NAME} v${VERSION} to ${dest}`);
  } catch (err) {
    console.error(`Failed to download ${BIN_NAME}: ${err.message}`);
    console.error(
      `\nYou can build from source instead:\n  cargo install --git https://github.com/${REPO}.git`
    );
    process.exit(1);
  }
}

main();
