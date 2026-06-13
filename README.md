# EmulStick Desktop

Operator-side console for the EmulStick BLE HID emulator — a native **Windows + macOS** app (Tauri 2 + Svelte) that drives a USB-HID keyboard/mouse dongle over BLE and can globally intercept system shortcuts a browser never could.

See [`docs/plan.md`](docs/plan.md) for the full engineering design, and [`docs/protocol.md`](docs/protocol.md) / [`docs/ble-protocol.md`](docs/ble-protocol.md) for the wire format.

## Architecture

A **control plane** (Svelte frontend) is split from a **data plane** (Rust backend) so high-frequency input never crosses the JSON IPC bridge.

- **Frontend** (`src/`) — scan/connect/status UI, passthrough toggles, HDMI video (M4), debug report sender. Low-frequency commands/events only.
- **Backend** (`src-tauri/src/`):
  - `protocol/` — HID report encoders (keyboard 8B / mouse 6B), the `rdev::Key → HID usage` keymap, and the normative BLE UUIDs. Hardware-independent and fully unit-tested against the worked examples in `docs/protocol.md`.
  - `ble/` — `btleplug` scan/connect, Device Info readout, write-without-response to F801/F803, LED notifications.
  - `ipc/` — Tauri commands + events.
  - `input/` — `rdev::grab` thread + cursor lock (M2, stubbed).
  - `state.rs` — passthrough flags, lock state.

## Status

Milestone **M1 (BLE bring-up)** is implemented: scan, connect, read Device Info, and send hand-crafted keyboard/mouse reports from the debug panel. M2 (input pipeline) onward is scaffolded but not yet wired — see the milestone list in `docs/plan.md` §12.

## Prerequisites

- [Rust](https://rustup.rs) (stable) and the platform Tauri prerequisites.
- Node.js 18+ and npm.

## Development

```bash
npm install            # frontend deps + Tauri CLI
npm run tauri dev      # run the app (spawns Vite + the Rust backend)
```

### Useful commands

```bash
npm run build                  # build the Svelte frontend
npm run check                  # svelte-check (type-check)
cargo test --manifest-path src-tauri/Cargo.toml   # protocol byte-exact tests
```

## macOS permissions

Global input interception needs **System Settings → Privacy & Security → Accessibility** (and possibly **Input Monitoring**) granted to the app. BLE prompts on first use via `NSBluetoothAlwaysUsageDescription`. Unsigned builds lose the Accessibility grant on every update — sign/notarize for stable iteration (plan §5, M5).
