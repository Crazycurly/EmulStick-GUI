//! OS-level input hooking (plan §4.2, §6.3, §8).
//!
//! A single dedicated [`rdev::grab`] thread installs a global hook. Its
//! callback is synchronous and does no I/O (plan §8): it consults lock state +
//! passthrough flags, maps events to HID reports via [`crate::protocol`], and
//! pushes them onto a channel consumed by an async **writer task** that talks
//! to BLE and applies the §6.3 mouse coalescing/clamping.
//!
//! `rdev 0.5` has no API to stop `grab()` once installed, so the hook is
//! installed lazily on first lock-enter and then stays resident, becoming
//! *transparent* (returns the event unchanged) whenever lock mode is inactive
//! or the channel's passthrough flag is off.

mod platform;

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use rdev::{Button, Event, EventType, Key};
use tauri::{AppHandle, Manager};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::ipc::events;
use crate::protocol::mouse::button as mbtn;
use crate::protocol::{mouse, KeyboardState, KEYBOARD_RELEASE_ALL, MOUSE_RELEASE_ALL};
use crate::state::{AppState, PassthroughFlags};

/// Mouse flush cadence. Decouples ~1000 Hz input from the BLE pipe; should sit
/// near the negotiated BLE connection interval (plan §6.3 / §10).
const FLUSH_INTERVAL: Duration = Duration::from_millis(8);

/// Scroll throttle: emit one HID wheel tick per this many accumulated macOS
/// line-deltas. Higher = slower host scrolling. Tunable.
const SCROLL_DIVISOR: i32 = 4;

/// Messages from the (sync) grab thread to the (async) writer task.
enum InputMsg {
    /// A fully-rendered 8-byte keyboard report (latency-critical, sent now).
    Keyboard([u8; 8]),
    /// New mouse button bitmask (latency-sensitive, flushed immediately).
    Buttons(u8),
    /// Accumulated relative motion since the last event.
    Move { dx: i32, dy: i32 },
    /// Wheel ticks, folded into the next flush.
    Wheel(i32),
    /// Lock mode toggled (from the UI command or the Ctrl+Alt hotkey). The
    /// writer owns the canonical transition: it sets the flag, emits
    /// `lock_state`, and on exit sends the safe all-up reports.
    SetLock(bool),
}

/// Lock-free state shared between the grab callback and the rest of the app.
/// Reads in the hot callback path are plain atomic loads (no mutex).
pub struct InputShared {
    lock_active: AtomicBool,
    pass_keyboard: AtomicBool,
    pass_mouse: AtomicBool,

    /// Held keys/modifiers, used to render full keyboard reports.
    keyboard: Mutex<KeyboardState>,
    /// Current mouse button bitmask.
    buttons: AtomicU8,

    /// Ctrl/Alt held flags + edge latch for the lock hotkey.
    ctrl_down: AtomicBool,
    alt_down: AtomicBool,
    hotkey_engaged: AtomicBool,

    /// Whether a grab hook is currently installed and running. Set when the
    /// grab thread is spawned, cleared if `rdev::grab` returns an error, so a
    /// later lock-enter can re-install it (e.g. after Accessibility is granted
    /// — otherwise a one-time failure would wedge lock mode "on but blind").
    grab_active: AtomicBool,

    /// Sender to the writer task, populated once the pipeline starts.
    tx: OnceLock<UnboundedSender<InputMsg>>,
}

impl InputShared {
    pub fn new() -> Self {
        Self {
            lock_active: AtomicBool::new(false),
            pass_keyboard: AtomicBool::new(false),
            pass_mouse: AtomicBool::new(false),
            keyboard: Mutex::new(KeyboardState::new()),
            buttons: AtomicU8::new(0),
            ctrl_down: AtomicBool::new(false),
            alt_down: AtomicBool::new(false),
            hotkey_engaged: AtomicBool::new(false),
            grab_active: AtomicBool::new(false),
            tx: OnceLock::new(),
        }
    }

    pub fn is_locked(&self) -> bool {
        self.lock_active.load(Ordering::SeqCst)
    }

    pub fn set_locked(&self, active: bool) {
        self.lock_active.store(active, Ordering::SeqCst);
    }

    /// Whether the grab/writer pipeline has been started (so `request_lock`
    /// will actually be handled).
    pub fn pipeline_started(&self) -> bool {
        self.tx.get().is_some()
    }

    /// Mirror the passthrough flags so the grab callback can gate channels.
    pub fn set_passthrough(&self, keyboard: bool, mouse: bool) {
        self.pass_keyboard.store(keyboard, Ordering::SeqCst);
        self.pass_mouse.store(mouse, Ordering::SeqCst);
    }

    /// Request a lock-mode transition. No-op if the pipeline never started.
    pub fn request_lock(&self, active: bool) {
        if let Some(tx) = self.tx.get() {
            let _ = tx.send(InputMsg::SetLock(active));
        }
    }

    fn send(&self, msg: InputMsg) {
        if let Some(tx) = self.tx.get() {
            let _ = tx.send(msg);
        }
    }
}

impl Default for InputShared {
    fn default() -> Self {
        Self::new()
    }
}

/// Owns the grab thread + writer task handles and gates one-time startup.
pub struct InputController {
    shared: Arc<InputShared>,
    started: bool,
    _grab_thread: Option<std::thread::JoinHandle<()>>,
    _writer: Option<tauri::async_runtime::JoinHandle<()>>,
}

impl InputController {
    pub fn new(shared: Arc<InputShared>) -> Self {
        Self {
            shared,
            started: false,
            _grab_thread: None,
            _writer: None,
        }
    }

    /// Install the grab hook + writer task. The channel and async writer are
    /// created exactly once; the grab hook is (re)installed whenever one isn't
    /// already running. On macOS the grab fails until Accessibility is granted;
    /// that surfaces as an `error` event from the grab thread (plan §5), and
    /// because the thread clears `grab_active` on failure a later lock-enter can
    /// retry the install once the permission is granted.
    pub fn ensure_started(&mut self, app: &AppHandle) {
        // One-time: the channel + async writer task (owns BLE I/O + §6.3
        // coalescing). The `tx` OnceLock must only ever be set once.
        if !self.started {
            self.started = true;
            let (tx, rx) = unbounded_channel::<InputMsg>();
            let _ = self.shared.tx.set(tx);
            let app = app.clone();
            self._writer = Some(tauri::async_runtime::spawn(writer_loop(app, rx)));
        }

        // (Re)install the grab hook only if one isn't already resident. rdev's
        // `grab()` can't be stopped, so a live hook stays installed and just
        // becomes transparent when unlocked; a failed one (no Accessibility)
        // exits and clears the flag, so we can install a fresh hook here.
        if self.shared.grab_active.swap(true, Ordering::SeqCst) {
            return;
        }
        let shared = self.shared.clone();
        let app = app.clone();
        self._grab_thread = std::thread::Builder::new()
            .name("rdev-grab".into())
            .spawn(move || {
                let cb_shared = shared.clone();
                tracing::info!("installing global input grab hook");
                let result = rdev::grab(move |event| handle_event(&cb_shared, event));
                if let Err(e) = result {
                    // Re-arm so a later lock-enter retries the install.
                    shared.grab_active.store(false, Ordering::SeqCst);
                    tracing::error!(?e, "rdev::grab failed to install the global input hook");
                    #[cfg(target_os = "macos")]
                    let detail = "On macOS, enable this app under System Settings → \
                                  Privacy & Security → Accessibility, then relaunch.";
                    #[cfg(not(target_os = "macos"))]
                    let detail = "Another low-level input hook or security/anti-cheat \
                                  software may be blocking it; close it and relaunch.";
                    events::error(
                        &app,
                        "input_grab_failed",
                        format!("Could not install the global input hook ({e:?}). {detail}"),
                    );
                }
            })
            .ok();
    }
}

/// The grab callback (runs on the grab thread, must not block).
///
/// Returns `Some(event)` to pass the event through to the operator OS, or
/// `None` to consume it (so reserved combos like `Cmd+Tab` don't fire locally).
fn handle_event(shared: &InputShared, event: Event) -> Option<Event> {
    // One-time confirmation (dev builds only) that the OS actually delivers
    // events to our grab callback — the definitive check that the global hook is
    // live. If you never see this line after typing, the hook isn't firing.
    #[cfg(debug_assertions)]
    {
        static FIRST: std::sync::Once = std::sync::Once::new();
        FIRST.call_once(|| tracing::info!(?event, "grab callback received its first OS event"));
    }
    match &event.event_type {
        EventType::KeyPress(key) | EventType::KeyRelease(key) => {
            let pressed = matches!(event.event_type, EventType::KeyPress(_));

            // If this event completed the lock hotkey, consume it and forward
            // nothing — otherwise the modifier-down that triggered the toggle
            // would land on the host *after* the exit release-all and stick
            // there (Ctrl+Alt held forever).
            if track_hotkey(shared, key, pressed) {
                return None;
            }

            if !shared.is_locked() || !shared.pass_keyboard.load(Ordering::SeqCst) {
                return Some(event);
            }
            if let Ok(mut kb) = shared.keyboard.lock() {
                // macOS delivers Caps Lock as a `FlagsChanged` toggle, so rdev
                // reports a lone KeyPress when the LED turns on and a lone
                // KeyRelease when it turns off — one alternating event per
                // physical tap, tracking the lock state, not the key. Mirroring
                // that press/release would only hand the host a key-down edge
                // (what actually toggles its caps) every *other* tap, and leave
                // Caps Lock held in between. So emit a full down→up tap on every
                // event: one host toggle per physical press, never left held.
                // Windows/Linux send normal down/up edges, so Caps Lock there
                // takes the standard path below (this block is compiled out).
                #[cfg(target_os = "macos")]
                if matches!(key, Key::CapsLock) {
                    kb.apply(key, true);
                    shared.send(InputMsg::Keyboard(kb.report()));
                    kb.apply(key, false);
                    shared.send(InputMsg::Keyboard(kb.report()));
                    return None;
                }

                if kb.apply(key, pressed) {
                    shared.send(InputMsg::Keyboard(kb.report()));
                }
            }
            None
        }

        EventType::ButtonPress(button) | EventType::ButtonRelease(button) => {
            if !shared.is_locked() || !shared.pass_mouse.load(Ordering::SeqCst) {
                return Some(event);
            }
            let pressed = matches!(event.event_type, EventType::ButtonPress(_));
            if let Some(mask) = button_mask(button) {
                let prev = shared.buttons.load(Ordering::SeqCst);
                let next = if pressed { prev | mask } else { prev & !mask };
                shared.buttons.store(next, Ordering::SeqCst);
                shared.send(InputMsg::Buttons(next));
            }
            None
        }

        EventType::MouseMove { x, y } => {
            if !shared.is_locked() || !shared.pass_mouse.load(Ordering::SeqCst) {
                return Some(event);
            }
            // `relative_delta` normalises per platform: on macOS `x`/`y` are
            // already HID deltas (vendored rdev patch); on Windows they're the
            // absolute cursor point, which it converts to a delta + recentres
            // the frozen cursor (see `platform`).
            let (dx, dy) = platform::relative_delta(*x, *y);
            if dx != 0 || dy != 0 {
                shared.send(InputMsg::Move { dx, dy });
            }
            None
        }

        EventType::Wheel { delta_x: _, delta_y } => {
            if !shared.is_locked() || !shared.pass_mouse.load(Ordering::SeqCst) {
                return Some(event);
            }
            // Horizontal scroll (`delta_x`) is intentionally dropped: the F803
            // report carries a single signed wheel byte (vertical only), so
            // there's nowhere to forward it. Vertical-only by design (plan §IV).
            shared.send(InputMsg::Wheel(*delta_y as i32));
            None
        }
    }
}

/// Update Ctrl/Alt held state and toggle lock on the Ctrl+Alt rising edge
/// (default hotkey, plan §4.2). The hotkey is the guaranteed escape hatch.
///
/// Returns `true` if this event *triggered* a lock transition, so the caller
/// can consume it without forwarding (avoids leaving the modifier stuck).
fn track_hotkey(shared: &InputShared, key: &Key, pressed: bool) -> bool {
    match key {
        Key::ControlLeft | Key::ControlRight => shared.ctrl_down.store(pressed, Ordering::SeqCst),
        // The exit hotkey is Ctrl+Alt. On Windows an international-layout AltGr
        // synthesises LeftCtrl+RightAlt, which would otherwise trip this combo —
        // so off macOS only the *left* Alt counts and AltGr (`Key::AltGr` = right
        // Alt) is excluded. The Windows escape hatch is therefore Ctrl+LeftAlt.
        // macOS has no synthetic-AltGr issue, so either Alt works there.
        #[cfg(target_os = "macos")]
        Key::Alt | Key::AltGr => shared.alt_down.store(pressed, Ordering::SeqCst),
        #[cfg(not(target_os = "macos"))]
        Key::Alt => shared.alt_down.store(pressed, Ordering::SeqCst),
        _ => return false,
    }
    let both = shared.ctrl_down.load(Ordering::SeqCst) && shared.alt_down.load(Ordering::SeqCst);
    if both {
        // Rising edge of the combo: fire exactly once until both are released.
        // The hotkey is **exit-only** — it's the guaranteed escape hatch. Entry
        // is via the UI button, which avoids leaking a half-pressed modifier to
        // the operator OS (the Ctrl-down would reach the Mac before lock
        // engages, then its Ctrl-up would be consumed → stuck modifier).
        if !shared.hotkey_engaged.swap(true, Ordering::SeqCst) && shared.is_locked() {
            shared.request_lock(false);
            return true;
        }
    } else {
        shared.hotkey_engaged.store(false, Ordering::SeqCst);
    }
    false
}

fn button_mask(button: &Button) -> Option<u8> {
    match button {
        Button::Left => Some(mbtn::LEFT),
        Button::Right => Some(mbtn::RIGHT),
        Button::Middle => Some(mbtn::MIDDLE),
        // Extra buttons are platform-specific scancodes; map the common pair.
        Button::Unknown(1) => Some(mbtn::BUTTON4),
        Button::Unknown(2) => Some(mbtn::BUTTON5),
        Button::Unknown(_) => None,
    }
}

/// Async writer: forwards keyboard reports immediately and coalesces mouse
/// motion/wheel, flushing once per [`FLUSH_INTERVAL`] (plan §6.3).
async fn writer_loop(app: AppHandle, mut rx: UnboundedReceiver<InputMsg>) {
    // Resolve the managed state once, not on every report: the hot path runs at
    // the input rate (~1 kHz keys / 125 Hz mouse flushes), and `state()` is a
    // typemap lookup. `&state` deref-coerces to `&AppState` at each call site.
    let state = app.state::<AppState>();
    let mut acc_dx: i32 = 0;
    let mut acc_dy: i32 = 0;
    let mut acc_wheel: i32 = 0;
    let mut buttons: u8 = 0;
    let mut interval = tokio::time::interval(FLUSH_INTERVAL);
    // Keep an even cadence: if a BLE write runs long, skip the missed ticks
    // instead of firing them back-to-back (which bunches motion / stutters).
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            msg = rx.recv() => {
                let Some(msg) = msg else { break };
                match msg {
                    InputMsg::Keyboard(report) => {
                        if !write_keyboard(&state, &report).await {
                            reset_input_state(&state, &mut buttons, &mut acc_dx, &mut acc_dy, &mut acc_wheel);
                        }
                    }
                    InputMsg::Buttons(mask) => {
                        buttons = mask;
                        // Latency-sensitive: flush now, carrying pending motion.
                        if !flush_mouse(&state, buttons, &mut acc_dx, &mut acc_dy, &mut acc_wheel).await {
                            reset_input_state(&state, &mut buttons, &mut acc_dx, &mut acc_dy, &mut acc_wheel);
                        }
                    }
                    InputMsg::Move { dx, dy } => {
                        acc_dx += dx;
                        acc_dy += dy;
                    }
                    InputMsg::Wheel(w) => {
                        acc_wheel += w;
                    }
                    InputMsg::SetLock(active) => {
                        set_lock(&app, &state, active, &mut buttons, &mut acc_dx, &mut acc_dy, &mut acc_wheel).await;
                    }
                }
            }
            _ = interval.tick() => {
                // A sub-divisor wheel remainder alone isn't worth a packet; it
                // carries until it reaches a full tick or motion flushes.
                let pending = acc_dx != 0 || acc_dy != 0 || acc_wheel.abs() >= SCROLL_DIVISOR;
                if pending && !flush_mouse(&state, buttons, &mut acc_dx, &mut acc_dy, &mut acc_wheel).await {
                    reset_input_state(&state, &mut buttons, &mut acc_dx, &mut acc_dy, &mut acc_wheel);
                }
            }
        }
    }
}

/// Apply a lock-mode transition: set the flag, manage relative-mouse capture,
/// emit `lock_state`, and on exit send the safe all-up reports (plan §9).
async fn set_lock(
    app: &AppHandle,
    state: &AppState,
    active: bool,
    buttons: &mut u8,
    acc_dx: &mut i32,
    acc_dy: &mut i32,
    acc_wheel: &mut i32,
) {
    let shared = &state.input_shared;
    tracing::info!(
        active,
        grab_active = shared.grab_active.load(Ordering::SeqCst),
        pass_keyboard = shared.pass_keyboard.load(Ordering::SeqCst),
        pass_mouse = shared.pass_mouse.load(Ordering::SeqCst),
        "lock transition"
    );

    if active {
        shared.lock_active.store(true, Ordering::SeqCst);
        // Freeze + decouple the cursor so movement is captured relatively
        // (only when we're actually grabbing the mouse channel).
        if shared.pass_mouse.load(Ordering::SeqCst) {
            platform::set_cursor_capture(true);
        }
    } else {
        shared.lock_active.store(false, Ordering::SeqCst);
        platform::set_cursor_capture(false);
        if let Ok(mut kb) = shared.keyboard.lock() {
            kb.reset();
        }
        shared.buttons.store(0, Ordering::SeqCst);
        *buttons = 0;
        *acc_dx = 0;
        *acc_dy = 0;
        *acc_wheel = 0;
        write_keyboard(state, &KEYBOARD_RELEASE_ALL).await;
        write_mouse(state, &MOUSE_RELEASE_ALL).await;
    }

    events::lock_state(app, active);
}

/// Update relative-mouse capture when the mouse passthrough flag changes while
/// already locked.
fn refresh_cursor_capture(shared: &InputShared, mouse: bool) {
    if shared.is_locked() {
        platform::set_cursor_capture(mouse);
    }
}

/// React to a passthrough-flag change (plan §4.3 / §9), called from
/// `set_passthrough`. Always refreshes relative-mouse capture to match the
/// mouse flag; additionally, if a channel is switched **off while locked**,
/// it releases whatever that channel left held on the host and clears our
/// logical model — otherwise a key/button held at the moment the operator
/// disables its channel would stick pressed on the host forever (its release
/// event now passes straight through to the operator OS instead of the host).
pub fn on_passthrough_changed(shared: &InputShared, prev: PassthroughFlags, next: PassthroughFlags) {
    refresh_cursor_capture(shared, next.mouse);
    if !shared.is_locked() {
        return;
    }
    if prev.keyboard && !next.keyboard {
        if let Ok(mut kb) = shared.keyboard.lock() {
            kb.reset();
        }
        shared.send(InputMsg::Keyboard(KEYBOARD_RELEASE_ALL));
    }
    if prev.mouse && !next.mouse {
        shared.buttons.store(0, Ordering::SeqCst);
        // A zero-button, zero-motion flush clears any held button on the host.
        shared.send(InputMsg::Buttons(0));
    }
}

/// Unconditionally release relative-mouse capture (unfreeze + show the cursor).
/// Idempotent and safe to call even if capture was never enabled. Used on app
/// exit and panic so the operator's cursor is never left frozen if the app dies
/// while locked (plan §8).
pub fn release_cursor_capture() {
    platform::set_cursor_capture(false);
}

/// Whether the OS trusts this process to capture global input (macOS
/// Accessibility); always true elsewhere. See [`platform::accessibility_trusted`].
pub fn accessibility_trusted(prompt: bool) -> bool {
    platform::accessibility_trusted(prompt)
}

/// Emergency lock release for an **unexpected link drop** (plan §4.2/§8). Makes
/// the grab transparent and unfreezes the cursor *immediately*, synchronously,
/// without routing through the writer task — which may be blocked on the BLE
/// mutex held by the reconnecting `connect()`. The operator must never stay
/// trapped just because the dongle vanished.
///
/// No BLE release-all is sent (the link is already gone); the next successful
/// [`crate::ble::BleManager::connect`] sends release-all anyway. Caller emits
/// the `lock_state(false)` IPC event.
pub fn emergency_unlock(shared: &InputShared) {
    shared.lock_active.store(false, Ordering::SeqCst);
    if let Ok(mut kb) = shared.keyboard.lock() {
        kb.reset();
    }
    shared.buttons.store(0, Ordering::SeqCst);
    // Re-arm the Ctrl+Alt latch so the next combo is a clean rising edge.
    shared.hotkey_engaged.store(false, Ordering::SeqCst);
    platform::set_cursor_capture(false);
}

/// Flush coalesced motion/wheel as F803 packets. Returns `false` if any GATT
/// write failed (so the caller can drop to a safe state, plan §9).
async fn flush_mouse(
    state: &AppState,
    buttons: u8,
    acc_dx: &mut i32,
    acc_dy: &mut i32,
    acc_wheel: &mut i32,
) -> bool {
    // Throttle scroll: one tick per SCROLL_DIVISOR line-deltas, carrying the
    // remainder so slow scrolls aren't lost and fast ones don't race.
    let wheel = *acc_wheel / SCROLL_DIVISOR;
    *acc_wheel -= wheel * SCROLL_DIVISOR;
    let mut ok = true;
    for packet in mouse::split_reports(buttons, *acc_dx, *acc_dy, wheel) {
        ok &= write_mouse(state, &packet).await;
    }
    *acc_dx = 0;
    *acc_dy = 0;
    ok
}

/// Write a keyboard report; returns `false` if the GATT write failed.
async fn write_keyboard(state: &AppState, report: &[u8]) -> bool {
    state.ble.lock().await.write_keyboard(report).await.is_ok()
}

/// Write a mouse report; returns `false` if the GATT write failed.
async fn write_mouse(state: &AppState, report: &[u8]) -> bool {
    state.ble.lock().await.write_mouse(report).await.is_ok()
}

/// Drop to a safe logical state after a GATT write fails (plan §9): forget all
/// held keys/buttons and pending motion so no phantom press survives the
/// failure. The host itself is cleared by the release-all sent on the next
/// (re)connect ([`crate::ble::BleManager::connect`]); the link is down now, so
/// we can only fix our own model here.
fn reset_input_state(
    state: &AppState,
    buttons: &mut u8,
    acc_dx: &mut i32,
    acc_dy: &mut i32,
    acc_wheel: &mut i32,
) {
    let shared = &state.input_shared;
    if let Ok(mut kb) = shared.keyboard.lock() {
        kb.reset();
    }
    shared.buttons.store(0, Ordering::SeqCst);
    *buttons = 0;
    *acc_dx = 0;
    *acc_dy = 0;
    *acc_wheel = 0;
}
