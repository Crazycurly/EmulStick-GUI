# Project Engineering Design Document: EmulStick Desktop (Tauri)

## 1. Project Overview

This project rebuilds the original web-based EmulStick controller as a native **Windows and macOS** desktop application. The core objective is low-latency, hardware-level keyboard/mouse emulation that can globally intercept system shortcuts (e.g. `Alt+Tab`, the `Win`/`Cmd` key) — which a browser can never capture — and let users selectively pass keyboard, mouse, and video through to a controlled host.

The controlled host sees a standard USB HID keyboard/mouse (the EmulStick dongle); this app is the operator-side console that drives it over BLE while optionally showing the host's screen via an HDMI capture card.

### Non-Goals (initial scope)

* **Linux support.** The architecture below is intentionally Windows + macOS only. (`btleplug` and `rdev` do support Linux, but Wayland global-hook limitations and `uinput` plumbing are out of scope.)
* **Gamepad (`F802`), Pen/Consumer (`F804`), and Private Control (`F80F`) channels.** The protocol supports them (see `docs/protocol.md`), but v1 ships keyboard + mouse + video only. They are forward-compatible additions.
* **Recording/macros, multi-host switching, or audio capture.**

## 2. Core Technology Stack

| Domain | Technology/Library | Reason for Selection |
| --- | --- | --- |
| **Underlying Framework** | Tauri 2.x (Rust) | Extremely low memory footprint and a tiny bundle size (< 10 MB). Native access to BLE and OS input hooks. |
| **Frontend UI** | Svelte + Vite + TS | No Virtual DOM, compiles to lean JS; fast startup and a lightweight UI well-suited to a video-background layout. |
| **Bluetooth Comms** | `btleplug` (Rust) | Async BLE scanning + GATT writes. On Windows it uses WinRT; on macOS it uses CoreBluetooth. **Note:** CoreBluetooth never exposes a MAC address — devices are identified by an opaque, per-machine peripheral UUID (see §4.1). |
| **Input Interception** | `rdev` (Rust) | OS-level global hooks. Crucially, use `rdev::grab` (not `listen`) so intercepted events can be **consumed** locally instead of also acting on the operator machine (see §4.2). |
| **Video Capture** | WebRTC API (Frontend) | `getUserMedia` + `<video>` reads the UVC HDMI capture feed directly in the WebView with GPU-accelerated rendering — no frames cross the IPC boundary. |

### 2.1 Key Architecture Decisions (locked)

These are decided; reopen only if a listed validation assumption fails.

| Decision | Choice | Rationale | Rejected alternative |
| --- | --- | --- | --- |
| **BLE transport** | `btleplug` in the Rust backend | Input events originate in Rust (`rdev`), so keeping map→write in-process keeps high-frequency data off the IPC bus — the whole point of the control/data-plane split. | **Web Bluetooth (JS):** would force every keystroke/mouse flush across Rust→IPC→JS just to write BLE. Only viable with zero native code, which is impossible since global hooks require native code anyway. |
| **Input hook** | `rdev::grab` | Returning `None` from the callback *consumes* the event, so reserved combos (`Cmd+Tab`, `Win`) don't fire locally. | **`rdev::listen`:** observe-only; the key would still act on the operator machine. |
| **Video path** | `getUserMedia` in the WebView | Frames render GPU-direct in `<video>`, never touching IPC. | **Capture in Rust:** would require piping frames to the frontend — strictly worse. |
| **Mouse transport** | Coalesce + flush per BLE connection interval (§6.3) | Decouples 1000 Hz input from the BLE pipe; the single biggest factor in "feel." | **Forward every raw event:** overruns the link, adds jitter. |

**Assumptions to validate early (M1/M2), since they bound "best":**
1. The dongle honors a low BLE connection interval (mouse latency ceiling).
2. `rdev::grab` reliably *consumes* (not just observes) system combos on current macOS.

## 3. System Architecture and Data Flow

The system separates a **control plane (frontend)** from a **data plane (backend)** so that high-frequency input never touches the (relatively slow, JSON-serialized) IPC bridge.

### Module responsibilities

* **Frontend (Svelte)** — Control plane only. Triggers scans, shows connection/lock status, toggles the three passthrough flags, and renders the HDMI feed. It never sees per-frame cursor deltas.
* **Tauri IPC Channel** — Carries low-frequency **commands** (frontend→backend: `scan`, `connect`, `set_passthrough`, `enter_lock`, …) and low-frequency **events** (backend→frontend: device list, connection state, lock state, errors). It deliberately does **not** carry cursor coordinates or keystrokes.
* **Backend (Rust)** — Data plane. Maintains the BLE connection, runs the `rdev` global hook on a dedicated thread, maps OS input events to HID reports, coalesces/clamps them (§6.3), and writes them to the EmulStick characteristics.

### End-to-end data flow (lock mode active)

```
 OS hardware event                 (dedicated rdev thread)
   ┌──────────────┐   grab    ┌────────────────────────────┐
   │ key / mouse  │ ────────► │ 1. consume event (no local  │
   │  / wheel     │           │    side effect)             │
   └──────────────┘           │ 2. map → HID report bytes   │
                              │ 3. accumulate mouse delta   │
                              └──────────────┬──────────────┘
                                             │ mpsc channel
                                             ▼
                              ┌────────────────────────────┐
                              │ BLE writer task (async)     │
                              │ - clamp/split to ±2047      │
                              │ - flush at conn. interval   │
                              │ - write WITHOUT response    │
                              └──────────────┬──────────────┘
                                             ▼
                                    F801 (kbd) / F803 (mouse)
                                             ▼
                                    EmulStick dongle → Host PC
```

Keyboard events are latency-critical but low-rate, so they are forwarded immediately. Mouse-move events are high-rate, so they are accumulated and flushed once per BLE connection interval (§6.3).

## 4. Core Functional Requirements

### 4.1 Device Connection Management (BLE)

* Scan for and list nearby EmulStick devices. Filter the scan by the **Custom Service UUID `0000F800-…`** (and/or advertised name) to avoid showing unrelated BLE peripherals.
* Persist the **last connected device identifier** and auto-reconnect on startup.
  * **Important cross-platform caveat:** the identifier is *not* portable. On **Windows** `btleplug` exposes the Bluetooth device address (BD_ADDR); on **macOS** CoreBluetooth exposes only a system-generated peripheral UUID that is stable per machine but differs across machines and OS reinstalls. Store whatever `btleplug`'s `Peripheral::id()` returns and treat it as an opaque, machine-local key — do **not** assume it is a MAC address.
* Read **Device Information Service (`180A`)** characteristics (firmware revision `2A26`, system ID `2A23`) after connect to display device info and verify firmware compatibility (`1.2.x` / `1.3.0`, per `docs/protocol.md`).
* Surface connection state transitions (scanning / connecting / connected / disconnected / reconnecting) as IPC events.

### 4.2 OS-Level Input Hooking

* **State trigger:** the user enters/exits "Control Lock Mode" via a UI action or a configurable global hotkey (default `Ctrl+Alt`). Lock state is owned by the backend and broadcast to the UI.
* **Keystroke interception:** in lock mode the app intercepts *all* keyboard input — including reserved keys (`Win`/`Cmd`, `Alt+Tab`, etc.). Each event is consumed locally (no local side effect) and translated to the F801 report (§6.1).
  * This requires `rdev::grab`, which on Windows uses a low-level keyboard/mouse hook (`WH_KEYBOARD_LL`/`WH_MOUSE_LL`) and on macOS uses a `CGEventTap`. Returning `None` from the grab callback swallows the event.
  * **Known limits:** macOS will not deliver events from *secure input* contexts (e.g. password fields) or a handful of OS-reserved combos to a `CGEventTap`; some Windows combos (`Ctrl+Alt+Del`, the secure attention sequence) are unreachable from user space. These are documented limitations, not bugs.
* **Mouse locking:** lock the OS cursor to the window center and capture **relative** movement (Delta X/Y) plus wheel. Provide a clear visual indicator and a guaranteed escape hatch (the lock hotkey) so the operator can never get "trapped."

### 4.3 Passthrough Control

Three independent boolean flags, toggleable in real time. They gate what the backend forwards even while lock mode is active:

* `enable_keyboard` — intercept and send keyboard events.
* `enable_mouse` — intercept and send mouse/trackpad movement, buttons, and wheel.
* `enable_video` — activate and render the HDMI capture stream.

*Example: with only `enable_keyboard` on, the operator's mouse keeps driving local windows while keystrokes go to the controlled host.*

> Interaction with lock mode: when a flag is **off**, the corresponding events are **not grabbed** (they pass through to the operator OS normally). When **on**, they are grabbed and forwarded. This keeps local usability intact for whichever channel is disabled.

### 4.4 Video Integration & Display

* Enumerate available UVC capture devices via `navigator.mediaDevices.enumerateDevices()`.
* Let the user pick the HDMI capture card and request a resolution/frame-rate (e.g. 1080p@60) via `getUserMedia` constraints, rendering it as the main-window background `<video>`.
* Handle device hot-plug/removal and constraint failures gracefully (fall back to the highest supported mode).

## 5. Permissions & Security

Global input interception and BLE require explicit per-OS permissions:

* **Windows:** No special entitlement for low-level hooks, but some antivirus/EDR products flag global keyboard hooks. BLE requires the system Bluetooth radio to be on; no manifest capability is needed for classic desktop builds.
* **macOS:** 
  * Declare `NSBluetoothAlwaysUsageDescription` in `Info.plist` (BLE prompt on first use).
  * On first launch, guide the user to **System Settings → Privacy & Security → Accessibility** and enable the app — without it, `rdev`/`CGEventTap` cannot intercept system keys. Detect the missing permission and show actionable guidance rather than silently failing.
  * **Input Monitoring** may also be required depending on capture path; detect and prompt.
  * Sign and notarize the build; an unsigned app's Accessibility grant is revoked on every update.

## 6. Payload Protocol Definition

> Authoritative source: `docs/protocol.md` and `docs/ble-protocol.md`. The values below are normative — note there is **no Report ID byte**. All payloads are written with **Write Without Response** to the Custom Service `0000F800-…`.

| Channel | Characteristic UUID | Length |
| --- | --- | --- |
| Keyboard | `0000F801-0000-1000-8000-00805f9b34fb` | 8 bytes |
| Mouse | `0000F803-0000-1000-8000-00805f9b34fb` | 6 bytes |

### 6.1 Keyboard Report (F801, 8 bytes)

`[Modifiers, 0x00, Key1, Key2, Key3, Key4, Key5, Key6]`

* **Byte 0 — modifiers** (1 = pressed): bit0 `LeftCtrl`, bit1 `LeftShift`, bit2 `LeftAlt`, bit3 `LeftGui`, bit4 `RightCtrl`, bit5 `RightShift`, bit6 `RightAlt`, bit7 `RightGui`.
* **Byte 1 — reserved**, always `0x00`.
* **Bytes 2–7 — up to 6 simultaneous HID Usage IDs** (valid 4–106; `0` = empty slot). This is standard N-key rollover up to 6KRO; track the currently-held set and re-send the full array on every key down/up.
* The dongle can **Read/Notify** a 1-byte LED report on F801 (NumLock/CapsLock/ScrollLock); subscribe to reflect lock-key state in the UI.

*Example — `Win+Shift+S`:* `A0 00 16 00 00 00 00 00` while `S` is held (`A0` = RightGui+RightShift, `16` = usage ID for `s`).

### 6.2 Mouse Report (F803, 6 bytes)

`[Buttons, X_low, X_high, Y_low, Y_high, Wheel]`

* **Byte 0 — buttons** (1 = pressed): bit0 `Left`, bit1 `Right`, bit2 `Middle`, bit3 `Button4`, bit4 `Button5`.
* **Bytes 1–2 — relative X**, signed 16-bit **little-endian**, valid range **−2047..+2047**.
* **Bytes 3–4 — relative Y**, signed 16-bit little-endian, **−2047..+2047**.
* **Byte 5 — wheel**, signed 8-bit, **−127..+127**.

*Example — move (−2, +3):* `00 FE FF 03 00 00`.

### 6.3 High-Frequency Mouse Handling (critical implementation detail)

Naively forwarding every OS mouse event will overrun the BLE link and add jitter. The backend must:

1. **Coalesce:** accumulate raw deltas in the `rdev` thread; the BLE writer flushes the summed delta once per **connection interval** (typically ~7.5–30 ms negotiated). This decouples the (potentially 1000 Hz) input device from the BLE pipe.
2. **Clamp & split:** if an accumulated delta exceeds ±2047 on an axis, emit **multiple** F803 packets in sequence rather than truncating, so fast flicks track 1:1.
3. **De-duplicate button/wheel:** send button-state changes immediately (low rate, latency-sensitive); fold wheel ticks into the next flush, clamped to ±127.
4. **Write Without Response** lets several packets land per connection interval; cap the in-flight queue to avoid unbounded backpressure if the link stalls.

## 7. IPC Surface (Tauri commands & events)

Keep this surface small and low-frequency. Suggested shape:

**Commands (frontend → backend):**
* `scan_start()` / `scan_stop()`
* `connect(device_id)` / `disconnect()`
* `get_device_info() -> { firmware, model, … }`
* `set_passthrough({ keyboard, mouse, video })`
* `enter_lock()` / `exit_lock()`
* `set_lock_hotkey(combo)`

**Events (backend → frontend):**
* `devices_changed([{ id, name, rssi }])`
* `connection_state(state)` — `Scanning|Connecting|Connected|Disconnected|Reconnecting`
* `lock_state(active: bool)`
* `keyboard_leds({ num, caps, scroll })`
* `error({ code, message })`

## 8. Threading & Lifecycle Model

* **rdev grab thread:** dedicated OS thread (grab callbacks are synchronous and must not block). It only maps events and pushes them onto an `mpsc`/`crossbeam` channel — no BLE or I/O in the callback.
* **BLE async task:** Tokio task owning the `btleplug` peripheral; consumes the channel, applies §6.3, and performs GATT writes.
* **Tauri main/UI thread:** handles commands/events; never does blocking work.
* Lock-mode entry installs the grab; exit removes it. Disconnect or app exit must guarantee the grab is torn down and the cursor is unlocked (use RAII guards) so the operator's machine never gets stuck.

## 9. Reconnection & Error Handling

* Auto-reconnect with bounded exponential backoff on unexpected disconnect; surface `Reconnecting` state.
* If a GATT write fails, drop to a safe state: **release all keys/buttons** (send a zeroed keyboard report `00×8` and zeroed mouse report `00×6`) so nothing is left "stuck pressed" on the host, then attempt reconnect.
* On lock exit / disconnect / panic, always emit the all-keys-up + all-buttons-up reports.

## 10. Latency Budget (target)

| Stage | Target |
| --- | --- |
| OS event → grab callback | < 1 ms |
| Map + enqueue | < 0.1 ms |
| Channel → BLE flush (mouse) | ≤ 1 connection interval (~7.5–30 ms) |
| BLE air time (write w/o response) | a few ms |
| **End-to-end (keystroke)** | **< ~20 ms** |

Connection-interval negotiation dominates mouse latency; request the lowest interval the OS/dongle allow.

## 11. Suggested Project Structure

```
src/                    # Svelte frontend (control plane + video)
  lib/
  routes/
src-tauri/
  src/
    main.rs
    ble/                # btleplug connection pool, GATT writes
    input/              # rdev grab thread, OS-event → HID mapping
    protocol/           # report encoders (kbd 8B, mouse 6B), keymap table
    ipc/                # commands + events
    state.rs            # passthrough flags, lock state, RAII guards
docs/
  plan.md  protocol.md  ble-protocol.md
```

## 12. Milestones

1. **M1 — BLE bring-up:** scan, connect, read Device Info, send hand-crafted keyboard/mouse reports from a debug command. Validates §6 encoders against real hardware.
2. **M2 — Input pipeline:** `rdev::grab` thread, OS→HID keymap, lock-mode state machine, mouse coalescing/clamping (§6.3).
3. **M3 — Passthrough & UI:** three flags wired to grab installation; connection/lock status UI; persisted device + auto-reconnect.
4. **M4 — Video:** UVC enumeration, `getUserMedia` background, resolution selection.
5. **M5 — Hardening:** macOS permissions onboarding, signing/notarization, safe-state-on-failure, reconnection backoff.

## 13. Testing Strategy

* **Protocol unit tests:** byte-exact assertions for the encoders against every worked example in `docs/protocol.md` (e.g. `Win+Shift+S`, `Shift+Up+Right`, mouse `(−2,+3)`).
* **Keymap coverage:** table-driven test mapping `rdev::Key` → HID Usage ID for the full §Appendix table; assert no unmapped common keys.
* **Coalescing tests:** feed synthetic high-rate deltas, assert correct summation, ±2047 splitting, and flush cadence.
* **Manual hardware matrix:** Windows + macOS, verifying reserved-key interception, no stuck keys after abrupt disconnect, and cursor recovery on lock exit.

## 14. Risks & Open Questions

* **macOS Accessibility friction:** the grant is per-signed-build; CI/dev iteration needs a stable signing identity or the permission resets constantly.
* **BLE throughput ceiling:** if the negotiated connection interval is large, fast mouse motion may feel laggy; investigate requesting a faster interval and whether the dongle honors it.
* **`rdev` grab maturity on macOS:** confirm it reliably consumes (not just observes) events on current macOS; have a fallback plan if specific system combos leak through.
* **Capture-card identity vs BLE identity:** persisted device IDs are machine-local (§4.1) — document this so users aren't surprised that pairing doesn't roam between machines.
