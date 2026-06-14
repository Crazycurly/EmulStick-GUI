# Release: signing & notarization (macOS)

This covers building a **distributable, notarized** macOS release (plan §5, M5).
For day-to-day development the `cargo run` flow re-signs the debug binary with a
stable identity so the Accessibility grant persists across rebuilds — see
[`scripts/sign-and-run.sh`](../scripts/sign-and-run.sh); that path is unrelated
to the release signing below.

## Why notarize

Global input interception needs an **Accessibility** grant (System Settings →
Privacy & Security → Accessibility). macOS keys that grant to the app's code
signature. An ad-hoc / unsigned build's identity changes on every rebuild, so
the grant is revoked constantly (plan §14). A build signed with a stable
**Developer ID Application** identity and notarized keeps the grant across
updates, and runs without Gatekeeper warnings on other machines.

## Prerequisites

- A **Developer ID Application** certificate in the login keychain.
  > ⚠️ This is *not* the same as the "Apple Development" cert used by the dev
  > runner. As of this writing the build machine has only an Apple Development
  > cert (`security find-identity -v -p codesigning` shows no "Developer ID
  > Application: …" line), so the steps below cannot be exercised here until that
  > certificate is installed from the Apple Developer portal.
- An Apple ID with an **app-specific password**, or an **App Store Connect API
  key**, for the notarization submission.
- Your 10-character Apple Developer **Team ID**.

## What the build applies automatically

- **Hardened runtime** — Tauri enables it for signed macOS builds
  (`bundle.macOS.hardenedRuntime` defaults to `true`); required for notarization.
- **Entitlements** — [`src-tauri/Entitlements.plist`](../src-tauri/Entitlements.plist),
  wired via `bundle.macOS.entitlements`. It declares only the hardened-runtime–
  gated resources this app uses: `com.apple.security.device.bluetooth` (BLE) and
  `com.apple.security.device.camera` (UVC HDMI capture). Accessibility / global
  input hooks are governed by TCC, not an entitlement, so nothing is needed there.
- **Usage strings** — [`src-tauri/Info.plist`](../src-tauri/Info.plist) supplies
  `NSBluetoothAlwaysUsageDescription` and `NSCameraUsageDescription`, merged into
  the bundle's `Info.plist` at build time.

## Build & notarize

Tauri's bundler signs and (when notarization env vars are present) submits and
staples in one step. Set the signing identity and notarization credentials, then
build:

```bash
# Code-signing identity (exact string from `security find-identity -v -p codesigning`)
export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"

# Notarization — either an Apple ID + app-specific password …
export APPLE_ID="you@example.com"
export APPLE_PASSWORD="abcd-efgh-ijkl-mnop"   # app-specific password
export APPLE_TEAM_ID="497JXL6DDW"

# … or an App Store Connect API key instead of the three vars above:
# export APPLE_API_ISSUER="…"
# export APPLE_API_KEY="…"
# export APPLE_API_KEY_PATH="/path/to/AuthKey_XXXX.p8"

npm run tauri build
```

Output lands in `src-tauri/target/release/bundle/` (`.app` and `.dmg`). With the
notarization vars set, Tauri submits the bundle to Apple, waits for the ticket,
and staples it. Without them, the build is signed but **not** notarized (fine for
local use on the signing machine; Gatekeeper will warn elsewhere).

## Verify

```bash
codesign --verify --deep --strict --verbose=2 \
  src-tauri/target/release/bundle/macos/EmulStick.app
codesign -d --entitlements - \
  src-tauri/target/release/bundle/macos/EmulStick.app   # should list bluetooth + camera
spctl --assess --type execute --verbose \
  src-tauri/target/release/bundle/macos/EmulStick.app   # "accepted / source=Notarized Developer ID"
xcrun stapler validate \
  src-tauri/target/release/bundle/macos/EmulStick.app
```

## First-run permissions (what the user sees)

1. **Bluetooth** prompts on first BLE use (the `NSBluetooth…` string).
2. **Accessibility** is required before lock mode can capture input. The app
   detects a missing grant, pops macOS's grant dialog, and shows an onboarding
   card with an **Open Settings** button (plan §5). After granting, relaunch.
3. **Camera** prompts on first HDMI/UVC capture (the `NSCamera…` string).

## Windows

Windows builds must be produced **on Windows** — the macOS dev machine can't
cross-compile the WinRT (BLE), WebView2, and Win32 (cursor capture) paths, and
the bundler's resource compiler isn't available there.

### Build

On a Windows 10/11 machine with Node, Rust (MSVC host toolchain), and the
WebView2 runtime (preinstalled on Win11 and current Win10):

```powershell
npm ci
npm run tauri build
```

Outputs land in `src-tauri/target/release/bundle/`:

- **NSIS** installer → `nsis/EmulStick_<version>_x64-setup.exe`
- **WiX MSI** → `msi/EmulStick_<version>_x64_en-US.msi`

The installer uses `webviewInstallMode: downloadBootstrapper`
(`src-tauri/tauri.conf.json`), so it fetches the WebView2 runtime at install time
if the machine doesn't already have it.

### CI

[`.github/workflows/windows-build.yml`](../.github/workflows/windows-build.yml)
builds the same NSIS + MSI artifacts on `windows-latest` for every push to
`main`/the active branch, every `v*` tag, PRs to `main`, and manual dispatch.
Download them from the **Artifacts** section of the workflow run. This is also
the authoritative Windows compile check for changes authored on macOS.

### First-run & permissions

- **Bluetooth** must be enabled (system radio on). No manifest capability is
  needed for a classic desktop build, and there is **no Accessibility-style
  grant** — low-level keyboard/mouse hooks (`WH_KEYBOARD_LL`/`WH_MOUSE_LL`) work
  without one. The exit hotkey is **Ctrl + LeftAlt** (AltGr is excluded so it
  doesn't trip on international layouts).
- **Code-signing** is not required for hooks to work, but an unsigned binary may
  trip SmartScreen ("Windows protected your PC" → *More info* → *Run anyway*) or
  AV heuristics that flag global keyboard hooks. Authenticode signing is out of
  scope for now (would remove those warnings on other machines).
