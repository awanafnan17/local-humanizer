# Installation Guide

Local Humanizer ships as self-contained desktop installers bundling the Tauri v2 shell, `llama.cpp` `b12600` (`llama-server`), and the default `Qwen3-1.7B-Q4_K_M.gguf` (`1.03 GiB`, `Apache-2.0`) model—with **zero developer prerequisites** (no Python, Node.js, Rust, or external runtimes required).

---

## 1. Download & SHA-256 Verification

Download the installer for your platform and `SHA256SUMS.txt` from the GitHub Releases page of `https://github.com/awanafnan17/local-humanizer`.

Verify the installer checksum before opening:

- **macOS:**
  ```bash
  shasum -a 256 -c SHA256SUMS.txt --ignore-missing
  ```
- **Windows (PowerShell):**
  ```powershell
  Get-FileHash .\LocalHumanizer-Windows-Setup.exe -Algorithm SHA256
  # Compare output Hash against SHA256SUMS.txt
  ```

---

## 2. macOS Installation (`LocalHumanizer-macOS.dmg`)

- **Apple Silicon (`arm64`):** Uses `LocalHumanizer-macOS.dmg` with **Metal GPU** acceleration enabled by default (`macOS 12.0+`).
- **Intel (`x86_64`):** Uses `LocalHumanizer-macOS-x64.dmg` (CPU-only `-ngl 0`). *Note: Bare-metal Intel macOS runtime verification is currently **`BLOCKED / UNVERIFIED`** (cross-compiled in CI without physical Intel Mac hardware).*

### Steps
1. Double-click `LocalHumanizer-macOS.dmg` to mount the disk image.
2. Drag **Local Humanizer.app** into the **Applications** folder shortcut.
3. Eject the mounted disk image.
4. Launch **Local Humanizer** from `/Applications` or Launchpad.

### Apple Code-Signing & Notarization Caveat (`BLOCKED / UNVERIFIED`)
Local and CI macOS builds are **ad-hoc signed** (`codesign --force --deep --sign -`) because an Apple Developer ID certificate and Apple notarization credentials are not available (`BLOCKED / UNVERIFIED`). On first launch, macOS Gatekeeper may warn that the app is from an unidentified developer:
1. Click **Cancel** (or **Done**), then open **System Settings -> Privacy & Security -> Open Anyway**, **or**
2. Run `xattr -cr "/Applications/Local Humanizer.app"` in Terminal after verifying `SHA256SUMS.txt`.

---

## 3. Windows Installation (`LocalHumanizer-Windows-Setup.exe`)

- **Target:** Windows 10 / 11 (`x86_64`, CPU inference).
- **Installer Behavior (`src-tauri/tauri.conf.json`):** Per-user NSIS installer (`installMode: "currentUser"`, no admin elevation required) creating a **Desktop shortcut**, **Start Menu entry**, and clean **Uninstaller** (`uninstall.exe` removes the binary, bundled runtime/model, and shortcuts). WebView2 uses Tauri's standard `downloadBootstrapper` fallback if not already preinstalled (Windows 11 includes WebView2).
- **Platform Verification Caveat (`BLOCKED / UNVERIFIED`):** `LocalHumanizer-Windows-Setup.exe` is built on GitHub Actions `windows-latest`, while physical Windows native install, runtime execution, Job Object reaping, and uninstall remain **`BLOCKED / UNVERIFIED`** locally due to no Windows OS/hardware on the build host. Additionally, the `.exe` is unsigned (SmartScreen may show **"Windows protected your PC"** -> click **More info -> Run anyway** after verifying SHA-256).

---

## 4. First Launch & Offline Verification

1. Wait for the header status pill to transition from **Initializing** -> **Starting Runtime** -> green **Ready** (typically `~2–15 s` on Apple Silicon depending on system memory pressure).
2. Paste or import (`.txt`/`.md`/`.markdown`, `<= 2 MiB`) up to `3,000+` words into **Original Text**, select a Mode and Intensity, and click **Humanize** (or press `Cmd/Ctrl+Enter`).
3. **100% Offline Operation:** Normal rewriting requires zero internet connection. Internet access is used **only** if you optionally open **Settings** and click **Download** on an unbundled model (such as `Qwen3 4B Instruct 2507` for `>=16 GiB` RAM machines), which verifies HTTPS URLs and exact SHA-256 checksums before activation.
