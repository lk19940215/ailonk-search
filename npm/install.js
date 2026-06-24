#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");
const https = require("https");
const http = require("http");

const REPO = "lk19940215/ailonk-search";
const BIN_NAME = "ailonk-search";
const MAX_RETRIES = 3;
const MAX_REDIRECTS = 5;
const TIMEOUT_MS = 60_000;
const VERSION_TAG_RE = /^v?\d+\.\d+\.\d+(-[\w.-]+)?$/i;

function compareVersions(a, b) {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const na = pa[i] || 0;
    const nb = pb[i] || 0;
    if (na !== nb) return na - nb;
  }
  return 0;
}

function getPlatformTarget() {
  const map = {
    "darwin-arm64": "aarch64-apple-darwin",
    "darwin-x64": "x86_64-apple-darwin",
    "linux-x64": "x86_64-unknown-linux-gnu",
    "win32-x64": "x86_64-pc-windows-msvc",
  };
  const key = `${process.platform}-${process.arch}`;
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

function applyMirror(url) {
  const mirror = process.env.GITHUB_MIRROR;
  if (mirror) {
    return `${mirror.replace(/\/+$/, "")}/${url}`;
  }
  return url;
}

function normalizeVersionTag(input, source) {
  const tag = input.startsWith("v") ? input : `v${input}`;
  if (!VERSION_TAG_RE.test(tag)) {
    throw new Error(`Invalid version tag from ${source}: ${input}`);
  }
  return tag;
}

function request(url, options, redirectsLeft = MAX_REDIRECTS) {
  return new Promise((resolve, reject) => {
    const client = url.startsWith("https:") ? https : http;
    const req = client.get(url, options, (res) => {
      if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
        if (redirectsLeft <= 0) {
          return reject(new Error(`Too many redirects for ${url}`));
        }
        const next = new URL(res.headers.location, url).href;
        return request(next, options, redirectsLeft - 1).then(resolve, reject);
      }
      resolve(res);
    });
    req.on("timeout", () => {
      req.destroy();
      reject(new Error(`Timeout fetching ${url}`));
    });
    req.on("error", reject);
  });
}

function fetchJSON(url) {
  return new Promise((resolve, reject) => {
    request(
      url,
      {
        headers: {
          "User-Agent": "ailonk-search-npm",
          Accept: "application/vnd.github.v3+json",
        },
        timeout: TIMEOUT_MS,
      }
    ).then(
      (res) => {
        if (res.statusCode !== 200) {
          return reject(new Error(`HTTP ${res.statusCode} for ${url}`));
        }
        const chunks = [];
        res.on("data", (c) => chunks.push(c));
        res.on("end", () => {
          try {
            resolve(JSON.parse(Buffer.concat(chunks).toString()));
          } catch {
            reject(new Error(`Invalid JSON from ${url}`));
          }
        });
        res.on("error", reject);
      },
      reject
    );
  });
}

function download(url) {
  return new Promise((resolve, reject) => {
    request(url, { headers: { "User-Agent": "ailonk-search-npm" }, timeout: TIMEOUT_MS }).then(
      (res) => {
        if (res.statusCode !== 200) {
          return reject(new Error(`HTTP ${res.statusCode} for ${url}`));
        }
        const total = parseInt(res.headers["content-length"], 10) || 0;
        const chunks = [];
        let received = 0;
        res.on("data", (c) => {
          chunks.push(c);
          received += c.length;
          if (total > 0 && process.stderr.isTTY) {
            const pct = ((received / total) * 100).toFixed(0);
            const mb = (received / 1048576).toFixed(1);
            process.stderr.write(`\r  Progress: ${mb}MB / ${(total / 1048576).toFixed(1)}MB (${pct}%)`);
          }
        });
        res.on("end", () => {
          if (process.stderr.isTTY) process.stderr.write("\n");
          resolve(Buffer.concat(chunks));
        });
        res.on("error", reject);
      },
      reject
    );
  });
}

async function downloadWithRetry(url, retries = MAX_RETRIES) {
  for (let i = 1; i <= retries; i++) {
    try {
      return await download(url);
    } catch (err) {
      if (i === retries) throw err;
      const wait = i * 2000;
      console.log(`  Retry ${i}/${retries - 1} in ${wait / 1000}s: ${err.message}`);
      await new Promise((r) => setTimeout(r, wait));
    }
  }
}

async function getTargetVersion() {
  const pinned = process.env.AILONK_VERSION;
  if (pinned) {
    const tag = normalizeVersionTag(pinned, "AILONK_VERSION");
    console.log(`Using pinned version: ${tag} (AILONK_VERSION)`);
    return tag;
  }
  const apiUrl = `https://api.github.com/repos/${REPO}/releases/latest`;
  const tryUrl = process.env.GITHUB_MIRROR ? applyMirror(apiUrl) : apiUrl;
  try {
    const release = await fetchJSON(tryUrl);
    if (!release.tag_name) {
      throw new Error("release response missing tag_name");
    }
    return normalizeVersionTag(release.tag_name, "GitHub API");
  } catch (err) {
    const fallback = normalizeVersionTag(require("./package.json").version, "package.json");
    console.log(`  Could not fetch latest release (${err.message}), falling back to ${fallback}`);
    return fallback;
  }
}

async function main() {
  const target = getPlatformTarget();
  const assetName = getBinaryName(target);
  const tag = await getTargetVersion();
  const version = tag.replace(/^v/, "");

  const binDir = __dirname;
  const dest = path.join(binDir, process.platform === "win32" ? `${BIN_NAME}.exe` : BIN_NAME);
  const versionFile = path.join(binDir, ".installed-version");

  if (fs.existsSync(dest) && fs.existsSync(versionFile)) {
    const installed = fs.readFileSync(versionFile, "utf8").trim();
    if (installed === version) {
      console.log(`${BIN_NAME} v${version} already installed`);
      return;
    }
    if (compareVersions(installed, version) > 0) {
      console.log(`${BIN_NAME} v${installed} is newer than target v${version}, skipping`);
      return;
    }
  }

  const rawUrl = `https://github.com/${REPO}/releases/download/${tag}/${assetName}`;
  const url = applyMirror(rawUrl);

  console.log(`Downloading ${BIN_NAME} ${tag} for ${target}...`);
  console.log(`  ${url}`);

  try {
    const data = await downloadWithRetry(url);
    if (!data.length) {
      throw new Error("Downloaded file is empty");
    }

    const tmp = `${dest}.tmp`;
    fs.writeFileSync(tmp, data);
    if (process.platform !== "win32") {
      fs.chmodSync(tmp, 0o755);
    }
    fs.renameSync(tmp, dest);
    fs.writeFileSync(versionFile, version);
    console.log(`Installed ${BIN_NAME} v${version} to ${dest}`);
  } catch (err) {
    const tmp = `${dest}.tmp`;
    if (fs.existsSync(tmp)) fs.unlinkSync(tmp);
    console.error(`Failed to download ${BIN_NAME}: ${err.message}`);
    if (!process.env.GITHUB_MIRROR) {
      console.error(`\nTip: In China, set GITHUB_MIRROR to speed up downloads:`);
      console.error(`  GITHUB_MIRROR=https://ghproxy.com npm install ailonk-search`);
    }
    console.error(`\nOr build from source:\n  cargo install --git https://github.com/${REPO}.git`);
    process.exit(1);
  }
}

main().then(
  () => process.exit(0),
  (err) => {
    console.error(err.message);
    process.exit(1);
  }
);
