#!/usr/bin/env node
import { createHash } from "node:crypto";
import fs from "node:fs";
import fsp from "node:fs/promises";
import path from "node:path";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const ROOT_DIR = path.resolve(__dirname, "..");
const MODELS_DIR = path.join(ROOT_DIR, "src-tauri", "resources", "models");
const MANIFEST_PATH = path.join(MODELS_DIR, "models.json");

function parseArgs(argv) {
  const opts = {
    modelId: "qwen3-1.7b-q4_k_m",
    destDir: MODELS_DIR,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--model") {
      const next = argv[i + 1];
      if (!next || next.startsWith("-")) {
        throw new Error("Missing value for --model");
      }
      opts.modelId = next;
      i += 1;
    } else if (arg.startsWith("--model=")) {
      const val = arg.slice("--model=".length);
      if (!val) throw new Error("Missing value for --model=");
      opts.modelId = val;
    } else if (arg === "--dest") {
      const next = argv[i + 1];
      if (!next || next.startsWith("-")) {
        throw new Error("Missing value for --dest");
      }
      opts.destDir = path.resolve(next);
      i += 1;
    } else if (arg === "--help" || arg === "-h") {
      console.log("Usage: node scripts/fetch-model.mjs [--model <id>] [--dest <dir>]");
      process.exit(0);
    } else {
      throw new Error(`Unknown argument: ${arg}`);
    }
  }
  return opts;
}

async function computeSha256(filePath) {
  const hash = createHash("sha256");
  await new Promise((resolve, reject) => {
    const rs = fs.createReadStream(filePath);
    rs.on("data", (chunk) => hash.update(chunk));
    rs.on("error", reject);
    rs.on("end", resolve);
  });
  return hash.digest("hex");
}

async function verifyFile(filePath, expectedSize, expectedSha256) {
  if (!fs.existsSync(filePath)) return false;
  const stat = await fsp.stat(filePath);
  if (!stat.isFile() || stat.size !== expectedSize) return false;
  const digest = await computeSha256(filePath);
  return digest === expectedSha256.toLowerCase();
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  const raw = await fsp.readFile(MANIFEST_PATH, "utf8");
  const manifest = JSON.parse(raw);
  const entry = (manifest.models || []).find((m) => m.id === opts.modelId);
  if (!entry) {
    throw new Error(`Model '${opts.modelId}' not found in ${MANIFEST_PATH}`);
  }
  if (!entry.download_url.startsWith("https://")) {
    throw new Error(`Refusing non-HTTPS model URL: ${entry.download_url}`);
  }

  await fsp.mkdir(opts.destDir, { recursive: true });
  const destPath = path.join(opts.destDir, entry.filename);
  const partPath = `${destPath}.part`;

  if (await verifyFile(destPath, entry.size_bytes, entry.sha256)) {
    console.log(
      `Verified existing model ${entry.filename} (${entry.size_bytes} bytes, sha256:${entry.sha256})`,
    );
    return;
  }

  console.log(`Downloading ${entry.filename} from ${entry.download_url} ...`);
  const response = await fetch(entry.download_url, { redirect: "follow" });
  if (!response.ok || !response.body) {
    throw new Error(`Download failed (${response.status} ${response.statusText})`);
  }
  if (!response.url.startsWith("https://")) {
    throw new Error(`Redirected to non-HTTPS URL: ${response.url}`);
  }

  const outStream = fs.createWriteStream(partPath);
  await pipeline(Readable.fromWeb(response.body), outStream);

  const stat = await fsp.stat(partPath);
  if (stat.size !== entry.size_bytes) {
    await fsp.rm(partPath, { force: true });
    throw new Error(`Size mismatch: expected ${entry.size_bytes} bytes, got ${stat.size} bytes`);
  }

  const digest = await computeSha256(partPath);
  if (digest !== entry.sha256.toLowerCase()) {
    await fsp.rm(partPath, { force: true });
    throw new Error(`SHA-256 mismatch: expected ${entry.sha256}, got ${digest}`);
  }

  await fsp.rename(partPath, destPath);
  console.log(`Downloaded and verified ${destPath} (${stat.size} bytes, sha256:${digest})`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
