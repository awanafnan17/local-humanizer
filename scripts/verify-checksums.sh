#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

REQUIRE_RELEASE=0
if [[ "${1:-}" == "--require-release" ]]; then
  REQUIRE_RELEASE=1
fi

echo "==> Verifying bundled model against src-tauri/resources/models/models.json"
python3 - <<'PY'
import hashlib, json, os, sys

with open("src-tauri/resources/models/models.json", "r", encoding="utf-8") as f:
    manifest = json.load(f)

default_models = [m for m in manifest["models"] if m.get("is_default")]
if len(default_models) != 1:
    sys.exit(f"Expected exactly 1 default model, found {len(default_models)}")

m = default_models[0]
path = os.path.join("src-tauri/resources/models", m["filename"])
if not os.path.isfile(path):
    sys.exit(f"Missing default model file: {path}")

size = os.path.getsize(path)
if size != m["size_bytes"]:
    sys.exit(f"Size mismatch for {path}: got {size}, expected {m['size_bytes']}")

h = hashlib.sha256()
with open(path, "rb") as fp:
    for chunk in iter(lambda: fp.read(1024 * 1024), b""):
        h.update(chunk)
digest = h.hexdigest()
if digest != m["sha256"]:
    sys.exit(f"SHA-256 mismatch for {path}: got {digest}, expected {m['sha256']}")

print(f"OK: {path} ({size} bytes, sha256:{digest})")
PY

echo "==> Verifying runtime layout for host platform"
UNAME_S="$(uname -s)"
UNAME_M="$(uname -m)"
if [[ "$UNAME_S" == "Darwin" && "$UNAME_M" == "arm64" ]]; then
  RUNTIME_DIR="src-tauri/resources/runtime/macos-arm64"
  REQUIRED=(
    "llama-server"
    "libllama-server-impl.dylib"
    "libllama-common.1.dylib"
    "libmtmd.1.dylib"
    "libllama.1.dylib"
    "libggml.1.dylib"
    "libggml-cpu.1.dylib"
    "libggml-blas.1.dylib"
    "libggml-metal.1.dylib"
    "libggml-rpc.1.dylib"
    "libggml-base.1.dylib"
  )
  for f in "${REQUIRED[@]}"; do
    [[ -s "$RUNTIME_DIR/$f" ]] || { echo "Missing runtime file: $RUNTIME_DIR/$f"; exit 1; }
  done
  [[ -x "$RUNTIME_DIR/llama-server" ]] || { echo "llama-server is not executable"; exit 1; }
  echo "OK: $RUNTIME_DIR (${#REQUIRED[@]} required runtime files verified)"
elif [[ "$UNAME_S" == "Darwin" && "$UNAME_M" == "x86_64" ]]; then
  RUNTIME_DIR="src-tauri/resources/runtime/macos-x64"
  REQUIRED=(
    "llama-server"
    "libllama-server-impl.dylib"
    "libllama-common.1.dylib"
    "libmtmd.1.dylib"
    "libllama.1.dylib"
    "libggml.1.dylib"
    "libggml-cpu.1.dylib"
    "libggml-blas.1.dylib"
    "libggml-rpc.1.dylib"
    "libggml-base.1.dylib"
  )
  for f in "${REQUIRED[@]}"; do
    [[ -s "$RUNTIME_DIR/$f" ]] || { echo "Missing runtime file: $RUNTIME_DIR/$f"; exit 1; }
  done
  [[ -x "$RUNTIME_DIR/llama-server" ]] || { echo "llama-server is not executable"; exit 1; }
  echo "OK: $RUNTIME_DIR (${#REQUIRED[@]} required runtime files verified)"
else
  echo "Skipping macOS runtime layout check on $UNAME_S/$UNAME_M"
fi

echo "==> Verifying notices parity"
cmp -s THIRD_PARTY_NOTICES.md src-tauri/resources/notices/THIRD_PARTY_NOTICES.md || {
  echo "ERROR: THIRD_PARTY_NOTICES.md differs from src-tauri/resources/notices/THIRD_PARTY_NOTICES.md"
  exit 1
}
echo "OK: THIRD_PARTY_NOTICES.md byte-identical to bundled copy"

if [[ -f "dist-release/SHA256SUMS.txt" ]]; then
  echo "==> Verifying dist-release/SHA256SUMS.txt"
  (cd dist-release && shasum -a 256 -c SHA256SUMS.txt)
elif [[ "$REQUIRE_RELEASE" -eq 1 ]]; then
  echo "ERROR: dist-release/SHA256SUMS.txt not found"
  exit 1
else
  echo "INFO: dist-release/SHA256SUMS.txt not present yet (skipping)"
fi
