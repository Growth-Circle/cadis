#!/usr/bin/env node

"use strict";

const { execSync } = require("child_process");
const fs = require("fs");
const https = require("https");
const os = require("os");
const path = require("path");

const REPO = "Growth-Circle/cadis";
const BINARIES = ["cadis", "cadisd"];

const TARGETS = {
  "linux-x64": "x86_64-unknown-linux-gnu",
  "linux-arm64": "aarch64-unknown-linux-gnu",
  "darwin-x64": "x86_64-apple-darwin",
  "darwin-arm64": "aarch64-apple-darwin",
  "win32-x64": "x86_64-pc-windows-msvc",
};

function getTarget() {
  const key = `${os.platform()}-${os.arch()}`;
  const target = TARGETS[key];
  if (!target) {
    console.error(`cadis: unsupported platform ${key}`);
    console.error(`Supported: ${Object.keys(TARGETS).join(", ")}`);
    process.exit(0); // exit 0 so npm install doesn't fail
  }
  return target;
}

function getVersion() {
  const pkg = require("../package.json");
  return pkg.version;
}

function binDir() {
  return path.join(__dirname, "..", "native");
}

function download(url) {
  return new Promise((resolve, reject) => {
    const get = (u) => {
      https.get(u, { headers: { "User-Agent": "cadis-npm" } }, (res) => {
        if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
          return get(res.headers.location);
        }
        if (res.statusCode !== 200) {
          return reject(new Error(`HTTP ${res.statusCode} for ${u}`));
        }
        const chunks = [];
        res.on("data", (c) => chunks.push(c));
        res.on("end", () => resolve(Buffer.concat(chunks)));
        res.on("error", reject);
      }).on("error", reject);
    };
    get(url);
  });
}

async function main() {
  const target = getTarget();
  const version = getVersion();
  const ext = os.platform() === "win32" ? ".exe" : "";
  const dir = binDir();

  // Check if binaries already exist with correct version
  const cadisPath = path.join(dir, `cadis${ext}`);
  if (fs.existsSync(cadisPath)) {
    try {
      const out = execSync(`"${cadisPath}" --version`, { encoding: "utf8", timeout: 5000 });
      if (out.includes(version)) {
        return; // already installed
      }
    } catch {}
  }

  const base = `https://github.com/${REPO}/releases/download/v${version}`;

  fs.mkdirSync(dir, { recursive: true });

  for (const name of BINARIES) {
    const url = `${base}/${name}-${target}${ext}`;
    process.stdout.write(`Downloading ${name} v${version} for ${target}...`);
    try {
      const buf = await download(url);
      const dest = path.join(dir, `${name}${ext}`);
      fs.writeFileSync(dest, buf, { mode: 0o755 });
      console.log(" ok");
    } catch (err) {
      console.log(" failed");
      console.error(`  ${err.message}`);
      console.error(`  You can download manually from: ${url}`);
      process.exit(0); // don't break npm install
    }
  }
}

main();
