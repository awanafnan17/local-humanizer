#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "==> 1. npm audit (--audit-level=high)"
npm audit --audit-level=high

echo "==> 2. cargo audit (src-tauri/Cargo.lock)"
if cargo audit --no-fetch --file src-tauri/Cargo.lock; then
  echo "cargo audit passed"
else
  echo "cargo audit --no-fetch failed; retrying with fetch"
  cargo audit --file src-tauri/Cargo.lock
fi

echo "==> 3. Notices parity & required license files"
cmp -s THIRD_PARTY_NOTICES.md src-tauri/resources/notices/THIRD_PARTY_NOTICES.md || {
  echo "ERROR: Notice copies out of sync"
  exit 1
}
for f in LICENSE-Apache-2.0.txt LICENSE-llama.cpp.txt LICENSE-LLVM-OpenMP.txt; do
  [[ -s "src-tauri/resources/notices/$f" ]] || { echo "Missing notice file: $f"; exit 1; }
done
echo "OK: Notice parity & bundled license texts verified"

echo "==> 4. Banned cloud/telemetry/upstream-leak scan"
BANNED_REGEX='stealth''humanizer|gpt''zero|lite''llm|rudra''-free|academic''-inversion|translate''\.googleapis|sk-live''-[A-Za-z0-9]{10,}|sk-ant''-[A-Za-z0-9]{10,}|AIza''[0-9A-Za-z_-]{20,}|AKIA''[0-9A-Z]{16}|ghp_''[A-Za-z0-9]{20,}|gho_''[A-Za-z0-9]{20,}'
if grep -rnE "$BANNED_REGEX" \
  src src-tauri/src src-tauri/tests scripts index.html package.json; then
  echo "ERROR: Banned token or secret detected in source!"
  exit 1
fi
echo "OK: Zero banned cloud/telemetry/secret strings in source"

echo "==> 5. Git tracked-files hygiene guard"
if git ls-files | grep -E '^(dist-release/|node_modules/|src-tauri/target/|dist/|\.agents/)|\.gguf$|src-tauri/resources/runtime/(macos-|win-|linux-)'; then
  echo "ERROR: Forbidden binary or build artifact is tracked in git!"
  exit 1
fi
echo "OK: Zero forbidden tracked files in git"
