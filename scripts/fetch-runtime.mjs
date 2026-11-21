#!/usr/bin/env node
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import fsp from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const ROOT_DIR = path.resolve(__dirname, "..");
const RUNTIME_ROOT = path.join(ROOT_DIR, "src-tauri", "resources", "runtime");
const NOTICES_ROOT = path.join(ROOT_DIR, "src-tauri", "resources", "notices");

const LLAMA_CPP_TAG = "b12600";

const MANIFEST = {
  "macos-arm64": {
    url: `https://github.com/ggml-org/llama.cpp/releases/download/${LLAMA_CPP_TAG}/llama-${LLAMA_CPP_TAG}-bin-macos-arm64.tar.gz`,
    size: 16578695,
    sha256: "9ced6f8cbce915968a865ded7718c2c4d14a69876bf9abf544533aec9b76e975",
    format: "tar.gz",
    stripComponents: 1,
    ext: ".tar.gz",
  },
  "macos-x64": {
    url: `https://github.com/ggml-org/llama.cpp/releases/download/${LLAMA_CPP_TAG}/llama-${LLAMA_CPP_TAG}-bin-macos-x64.tar.gz`,
    size: 12364702,
    sha256: "1a76c5393ebf51ee8e067de9042fe628303b976be639912a9bcf80feb71044af",
    format: "tar.gz",
    stripComponents: 1,
    ext: ".tar.gz",
  },
  "win-cpu-x64": {
    url: `https://github.com/ggml-org/llama.cpp/releases/download/${LLAMA_CPP_TAG}/llama-${LLAMA_CPP_TAG}-bin-win-cpu-x64.zip`,
    size: 25012254,
    sha256: "a31a14f1630408422228d02ff2a3fc41c06a6b3b3849d8d2782c37f0f6e30951",
    format: "zip",
    stripComponents: 0,
    ext: ".zip",
  },
};

function detectHostPlatform() {
  if (process.platform === "darwin" && process.arch === "arm64") {
    return "macos-arm64";
  }
  if (process.platform === "darwin" && process.arch === "x64") {
    return "macos-x64";
  }
  if (process.platform === "win32" && process.arch === "x64") {
    return "win-cpu-x64";
  }
  throw new Error(`Unsupported host platform: ${process.platform}/${process.arch}. Pass --platform <id>.`);
}

function parseArgs(argv) {
  const opts = {
    platform: null,
    syncNotices: false,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--platform") {
      opts.platform = argv[i + 1] ?? null;
      i += 1;
    } else if (arg.startsWith("--platform=")) {
      opts.platform = arg.slice("--platform=".length);
    } else if (arg === "--sync-notices") {
      opts.syncNotices = true;
    } else if (arg === "--help" || arg === "-h") {
      console.log(
        "Usage: node scripts/fetch-runtime.mjs [--platform macos-arm64|macos-x64|win-cpu-x64|all] [--sync-notices]",
      );
      process.exit(0);
    } else {
      throw new Error(`Unknown argument: ${arg}`);
    }
  }
  return opts;
}

async function downloadAndVerify(entry, tmpPath) {
  console.log(`Downloading ${entry.url} ...`);
  const response = await fetch(entry.url, { redirect: "follow" });
  if (!response.ok || !response.body) {
    throw new Error(`Download failed (${response.status} ${response.statusText}) for ${entry.url}`);
  }

  const outStream = fs.createWriteStream(tmpPath);
  await pipeline(Readable.fromWeb(response.body), outStream);

  const stat = await fsp.stat(tmpPath);
  if (stat.size !== entry.size) {
    throw new Error(`Size mismatch: expected ${entry.size} bytes, got ${stat.size} bytes`);
  }

  const hash = createHash("sha256");
  await new Promise((resolve, reject) => {
    const rs = fs.createReadStream(tmpPath);
    rs.on("data", (chunk) => hash.update(chunk));
    rs.on("error", reject);
    rs.on("end", resolve);
  });

  const digest = hash.digest("hex");
  if (digest !== entry.sha256) {
    throw new Error(`SHA256 mismatch: expected ${entry.sha256}, got ${digest}`);
  }
  console.log(`Verified ${stat.size} bytes (sha256:${digest})`);
}

function extractArchive(tmpPath, destDir, entry) {
  if (entry.format === "tar.gz") {
    execFileSync("tar", [
      "-xzf",
      tmpPath,
      "-C",
      destDir,
      `--strip-components=${entry.stripComponents}`,
    ]);
    return;
  }

  if (entry.format === "zip") {
    if (process.platform === "win32") {
      try {
        execFileSync("tar", ["-xf", tmpPath, "-C", destDir]);
      } catch {
        execFileSync("powershell", [
          "-NoProfile",
          "-Command",
          `Expand-Archive -LiteralPath '${tmpPath.replace(/'/g, "''")}' -DestinationPath '${destDir.replace(/'/g, "''")}' -Force`,
        ]);
      }
    } else {
      execFileSync("unzip", ["-o", "-q", tmpPath, "-d", destDir]);
    }
    return;
  }

  throw new Error(`Unsupported archive format: ${entry.format}`);
}

async function fetchPlatform(platformId) {
  const entry = MANIFEST[platformId];
  if (!entry) {
    throw new Error(`Unknown platform '${platformId}'. Expected one of: ${Object.keys(MANIFEST).join(", ")}, all`);
  }

  const destDir = path.join(RUNTIME_ROOT, platformId);
  const tmpArchive = path.join(
    os.tmpdir(),
    `llama-${LLAMA_CPP_TAG}-${platformId}-${Date.now()}${entry.ext}`,
  );

  try {
    await downloadAndVerify(entry, tmpArchive);
    await fsp.rm(destDir, { recursive: true, force: true });
    await fsp.mkdir(destDir, { recursive: true });
    extractArchive(tmpArchive, destDir, entry);

    if (process.platform === "darwin") {
      try {
        execFileSync("xattr", ["-dr", "com.apple.quarantine", destDir]);
      } catch {
        // Best-effort quarantine removal
      }
    }

    const licenseSrc = path.join(destDir, "LICENSE");
    if (fs.existsSync(licenseSrc)) {
      await fsp.mkdir(NOTICES_ROOT, { recursive: true });
      await fsp.copyFile(licenseSrc, path.join(NOTICES_ROOT, "LICENSE-llama.cpp.txt"));
    }

    const openmpSrc = path.join(destDir, "LICENSE-LLVM-OpenMP");
    if (fs.existsSync(openmpSrc)) {
      await fsp.mkdir(NOTICES_ROOT, { recursive: true });
      await fsp.copyFile(openmpSrc, path.join(NOTICES_ROOT, "LICENSE-LLVM-OpenMP.txt"));
    }

    console.log(`Extracted ${platformId} runtime into ${destDir}`);
  } finally {
    await fsp.rm(tmpArchive, { force: true }).catch(() => {});
  }
}

async function syncWindowsOpenMpNotice() {
  const entry = MANIFEST["win-cpu-x64"];
  const tmpZip = path.join(
    os.tmpdir(),
    `llama-${LLAMA_CPP_TAG}-win-notices-${Date.now()}.zip`,
  );
  const tmpExtract = path.join(
    os.tmpdir(),
    `llama-${LLAMA_CPP_TAG}-win-notices-dir-${Date.now()}`,
  );

  try {
    await downloadAndVerify(entry, tmpZip);
    await fsp.mkdir(tmpExtract, { recursive: true });
    if (process.platform === "win32") {
      execFileSync("tar", ["-xf", tmpZip, "-C", tmpExtract, "LICENSE-LLVM-OpenMP"]);
    } else {
      execFileSync("unzip", ["-o", "-q", tmpZip, "LICENSE-LLVM-OpenMP", "-d", tmpExtract]);
    }
    await fsp.mkdir(NOTICES_ROOT, { recursive: true });
    await fsp.copyFile(
      path.join(tmpExtract, "LICENSE-LLVM-OpenMP"),
      path.join(NOTICES_ROOT, "LICENSE-LLVM-OpenMP.txt"),
    );
    console.log(`Synced LICENSE-LLVM-OpenMP.txt into ${NOTICES_ROOT}`);
  } finally {
    await fsp.rm(tmpZip, { force: true }).catch(() => {});
    await fsp.rm(tmpExtract, { recursive: true, force: true }).catch(() => {});
  }
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  const target = opts.platform ?? detectHostPlatform();
  const platforms = target === "all" ? Object.keys(MANIFEST) : [target];

  for (const platformId of platforms) {
    await fetchPlatform(platformId);
  }

  if (opts.syncNotices && !platforms.includes("win-cpu-x64")) {
    await syncWindowsOpenMpNotice();
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
