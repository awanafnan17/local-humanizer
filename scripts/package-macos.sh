#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "==> Checking free disk space on /System/Volumes/Data"
AVAIL_KB="$(df -k /System/Volumes/Data | awk 'NR==2 {print $4}')"
AVAIL_GB="$((AVAIL_KB / 1024 / 1024))"
echo "Available space: ${AVAIL_GB} GiB (${AVAIL_KB} KiB)"
if [[ "$AVAIL_KB" -lt 8388608 ]]; then
  echo "ERROR: At least 8 GiB free disk space required before packaging."
  exit 1
fi

echo "==> Verifying runtime, model, and notices before build"
bash scripts/verify-checksums.sh

echo "==> Running frontend typecheck and production build"
npm run typecheck
npm run build

echo "==> Running Tauri release build (app + dmg, CI=true to skip Finder AppleScript)"
CI=true npx tauri build --ci --bundles app,dmg

APP_PATH="src-tauri/target/release/bundle/macos/Local Humanizer.app"
DMG_BUNDLE_DIR="src-tauri/target/release/bundle/dmg"

if [[ ! -d "$APP_PATH" ]]; then
  echo "ERROR: Expected .app bundle not found at $APP_PATH"
  exit 1
fi

echo "==> Inspecting .app bundle resources"
RES_DIR="$APP_PATH/Contents/Resources/resources"
[[ -x "$RES_DIR/runtime/macos-arm64/llama-server" ]] || { echo "Missing bundled llama-server in .app"; exit 1; }
[[ -s "$RES_DIR/models/models.json" ]] || { echo "Missing bundled models.json in .app"; exit 1; }
[[ -s "$RES_DIR/models/Qwen3-1.7B-Q4_K_M.gguf" ]] || { echo "Missing bundled Qwen3-1.7B-Q4_K_M.gguf in .app"; exit 1; }
[[ -s "$RES_DIR/notices/THIRD_PARTY_NOTICES.md" ]] || { echo "Missing bundled THIRD_PARTY_NOTICES.md in .app"; exit 1; }

echo "==> Ad-hoc code signing .app bundle and verifying signature"
codesign --force --deep --sign - "$APP_PATH"
codesign --verify --deep --strict "$APP_PATH"

mkdir -p dist-release
OUT_DMG="dist-release/LocalHumanizer-macOS.dmg"

SRC_DMG="$(find "$DMG_BUNDLE_DIR" -maxdepth 1 -name '*.dmg' | head -n 1 || true)"
if [[ -z "$SRC_DMG" ]]; then
  echo "No DMG emitted by tauri-bundler; creating UDZO DMG via hdiutil"
  STAGING_DIR="$(mktemp -d /tmp/lh-dmg-stage.XXXXXX)"
  trap 'rm -rf "$STAGING_DIR"' EXIT
  cp -R "$APP_PATH" "$STAGING_DIR/"
  ln -s /Applications "$STAGING_DIR/Applications"
  hdiutil create -volname "Local Humanizer" -srcfolder "$STAGING_DIR" -ov -format UDZO "$OUT_DMG"
else
  # Rebuild DMG cleanly from the ad-hoc verified .app so the DMG holds the verified signature
  STAGING_DIR="$(mktemp -d /tmp/lh-dmg-stage.XXXXXX)"
  trap 'rm -rf "$STAGING_DIR"' EXIT
  cp -R "$APP_PATH" "$STAGING_DIR/"
  ln -s /Applications "$STAGING_DIR/Applications"
  hdiutil create -volname "Local Humanizer" -srcfolder "$STAGING_DIR" -ov -format UDZO "$OUT_DMG"
fi

echo "==> Generating dist-release/SHA256SUMS.txt"
(cd dist-release && shasum -a 256 LocalHumanizer-macOS.dmg > SHA256SUMS.txt)
bash scripts/verify-checksums.sh --require-release

echo "==> Packaging complete:"
ls -lh "$APP_PATH" "$OUT_DMG" dist-release/SHA256SUMS.txt
cat dist-release/SHA256SUMS.txt
