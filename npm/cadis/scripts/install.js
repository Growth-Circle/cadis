#!/usr/bin/env node

"use strict";

const { execSync } = require("child_process");
const crypto = require("crypto");
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

function checksumSha256(buffer) {
  return crypto.createHash("sha256").update(buffer).digest("hex");
}

function parseSha256File(text, expectedFileName) {
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line) continue;
    const [hash, ...rest] = line.split(/\s+/);
    if (!hash || !/^[a-f0-9]{64}$/i.test(hash)) continue;
    if (rest.length === 0) return hash.toLowerCase();
    const fileToken = rest.join(" ").replace(/^\*/, "");
    if (fileToken === expectedFileName) return hash.toLowerCase();
  }
  throw new Error(`Could not parse SHA256 for ${expectedFileName}`);
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
    const artifactName = `${name}-${target}${ext}`;
    const url = `${base}/${artifactName}`;
    const checksumUrl = `${url}.sha256`;
    process.stdout.write(`Downloading ${name} v${version} for ${target}...`);
    try {
      const [buf, checksumBuf] = await Promise.all([download(url), download(checksumUrl)]);
      const expected = parseSha256File(checksumBuf.toString("utf8"), artifactName);
      const actual = checksumSha256(buf);
      if (actual !== expected) {
        throw new Error(`Checksum mismatch for ${artifactName} (expected ${expected}, got ${actual})`);
      }
      const dest = path.join(dir, `${name}${ext}`);
      fs.writeFileSync(dest, buf, { mode: 0o755 });
      console.log(" ok");
    } catch (err) {
      console.log(" failed");
      console.error(`  ${err.message}`);
      console.error(`  You can download manually from: ${url}`);
      console.error(`  Checksum file: ${checksumUrl}`);
      process.exit(0); // don't break npm install
    }
  }
}

main();
