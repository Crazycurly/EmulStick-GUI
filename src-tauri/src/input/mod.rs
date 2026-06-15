//! OS-level input hooking (plan §4.2, §6.3, §8).
//!
//! A single dedicated [`rdev::grab`] thread installs a global hook. Its
//! callback is synchronous and does no I/O (plan §8): it consults lock state +
//! passthrough flags, maps keyboard events to HID reports via
//! [`crate::protocol`], and accumulates mouse motion/wheel into lock-free
//! atomics. An async **writer task** forwards keyboard reports immediately and
//! drains the accumulated motion once per [`FLUSH_INTERVAL`] (§6.3 coalescing),
//! writing to BLE via the shared [`crate::ble::ConnHandle`].
//!
//! `rdev 0.5` has no API to stop `grab()` once installed, so the hook is
//! installed lazily on first lock-enter and then stays resident, becoming
//! *transparent* (returns the event unchanged) whenever lock mode is inactive
//! or the channel's passthrough flag is off.

mod platform;

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering};
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

/// Messages from the (sync) grab thread to the (async) writer task. Mouse motion
/// and wheel are *not* messages — they coalesce into [`InputShared`] atomics so a
/// stalled writer can't accumulate an unbounded per-event backlog (plan §6.3).
enum InputMsg {
    /// A fully-rendered 8-byte keyboard report (latency-critical, sent now).
    Keyboard([u8; 8]),
    /// New mouse button bitmask (latency-sensitive, flushed immediately).
    Buttons(u8),
    /// Lock mode toggled (from the UI command or the Ctrl+Alt hotkey). The
    /// writer owns the canonical transition: it sets the flag, emits
    /// `lock_state`, and on exit sends the safe all-up reports.
    SetLock(bool),
    /// App is exiting (plan §8): flush the safe all-up reports as the *final*
    /// writes and stop the writer. Sent only after lock is forced inactive, so
    /// it is the last message the writer will ever see.
    Shutdown,
    /// The grab hook failed to install. Routed through the writer (not handled
    /// on the grab thread) so it is ordered *after* the `SetLock(true)` this same
    /// lock-enter already queued — the writer therefore emits `lock_state(true)`
    /// then `lock_state(false)`, leaving the UI reliably unlocked regardless of
    /// how the cross-thread `input_grab_failed` error event interleaves.
    GrabFailed,
}

/// Lock-free state shared between the grab callback and the rest of the app.
/// Reads in the hot callback path are plain atomic loads (no mutex).
pub struct InputShared {
    lock_active: AtomicBool,
    pass_keyboard: AtomicBool,
    pass_mouse: AtomicBool,

    /// OS of the **target system** being controlled. Forwarded keys swap Alt↔Win
    /// (Alt → ⌘, Win → ⌥; Ctrl stays Control) when this differs from the host OS,
    /// so the operator's modifiers line up across a Mac/PC keyboard mismatch.
    /// See [`remap_for_target`].
    target_mac: AtomicBool,

    /// Held keys/modifiers, used to render full keyboard reports.
    keyboard: Mutex<KeyboardState>,
    /// Current mouse button bitmask.
    buttons: AtomicU8,

    /// Coalesced relative mouse motion + wheel: the grab callback `fetch_add`s
    /// here and the writer drains it on the flush tick. Kept as atomics (not
    /// channel messages) so a stalled writer can't grow an unbounded backlog,
    /// and so motion is summed into one delta rather than replayed as a burst.
    acc_dx: AtomicI32,
    acc_dy: AtomicI32,
    acc_wheel: AtomicI32,

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
            target_mac: AtomicBool::new(false),
            keyboard: Mutex::new(KeyboardState::new()),
            buttons: AtomicU8::new(0),
            acc_dx: AtomicI32::new(0),
            acc_dy: AtomicI32::new(0),
            acc_wheel: AtomicI32::new(0),
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

    /// Select the **target system**'s OS for modifier remapping. The Alt↔Win swap
    /// (Alt→⌘, Win→⌥; Ctrl stays Control) only kicks in when it differs from the
    /// host OS — see [`remap_for_target`].
    ///
    /// If the swap rule changes *while locked*, a modifier held under the old
    /// mapping would later be released under the new one — clearing the wrong
    /// modifier bit and sticking the original down on the target. So on a real
    /// change while locked we release everything first (mirrors
    /// [`on_passthrough_changed`]), guaranteeing no held key spans two mappings.
    pub fn set_target_mac(&self, mac: bool) {
        let changed = self.target_mac.swap(mac, Ordering::SeqCst) != mac;
        if changed && self.is_locked() {
            if let Ok(mut kb) = self.keyboard.lock() {
                kb.reset();
            }
            self.send(InputMsg::Keyboard(KEYBOARD_RELEASE_ALL));
        }
    }

    /// Request a lock-mode transition. No-op if the pipeline never started.
    pub fn request_lock(&self, active: bool) {
        self.send(InputMsg::SetLock(active));
    }

    /// Ask the writer to flush a final all-up state and stop (app exit, plan §8).
    pub fn shutdown(&self) {
        self.send(InputMsg::Shutdown);
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
    writer: Option<tauri::async_runtime::JoinHandle<()>>,
}

impl InputController {
    pub fn new(shared: Arc<InputShared>) -> Self {
        Self {
            shared,
            started: false,
            _grab_thread: None,
            writer: None,
        }
    }

    /// Take the writer task handle so the exit path can await its clean stop.
    pub fn take_writer(&mut self) -> Option<tauri::async_runtime::JoinHandle<()>> {
        self.writer.take()
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
            self.writer = Some(tauri::async_runtime::spawn(writer_loop(app, rx)));
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
                    // Force lock back off through the writer so the canonical
                    // lock_state(false) is ordered after this enter's
                    // lock_state(true) — no reliance on cross-thread event order.
                    shared.send(InputMsg::GrabFailed);
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

                // Remap modifiers so they line up with the target system (Alt↔Win
                // when the host and target OS differ) before forwarding. The exit
                // hotkey was already tested on the raw key above, so this doesn't
                // affect it.
                let mapped = remap_for_target(key, shared.target_mac.load(Ordering::SeqCst));
                if kb.apply(&mapped, pressed) {
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
            // the frozen cursor (see `platform`). Accumulate into the shared
            // atomics; the writer drains them on its flush tick (§6.3).
            let (dx, dy) = platform::relative_delta(*x, *y);
            if dx != 0 {
                shared.acc_dx.fetch_add(dx, Ordering::SeqCst);
            }
            if dy != 0 {
                shared.acc_dy.fetch_add(dy, Ordering::SeqCst);
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
            shared.acc_wheel.fetch_add(*delta_y as i32, Ordering::SeqCst);
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

/// Remap modifier keys so they line up between the operator's keyboard and the
/// **target system** being controlled. A Mac keyboard's ⌘/⌥ row sits where a
/// PC's Win/Alt row does, so crossing the two requires swapping **Alt ↔ Win**:
///   * **Alt → Command(⌘)** / **Win → Option(⌥)** — driving a Mac from a PC,
///   * the mirror of that (the swap is its own inverse) — driving a PC from a Mac,
///   * **Ctrl → Control** — always left unchanged.
///
/// The swap is needed only when the target OS *differs* from the host OS this
/// build runs on; matching OSes already line up, so it's the identity. Applied
/// only to *forwarded* keys — the Ctrl+Alt exit hotkey is detected on the raw key
/// beforehand, so the escape hatch is unaffected by the swap.
fn remap_for_target(key: &Key, target_mac: bool) -> Key {
    remap_for_target_os(key, target_mac, cfg!(target_os = "macos"))
}

/// Host-OS-aware core of [`remap_for_target`], split out so the swap rule can be
/// unit-tested independent of the OS the test runner happens to be on.
fn remap_for_target_os(key: &Key, target_mac: bool, host_mac: bool) -> Key {
    // Same OS on both ends ⇒ the modifier rows already align ⇒ forward 1:1.
    if target_mac == host_mac {
        return key.clone();
    }
    match key {
        Key::Alt => Key::MetaLeft,
        Key::AltGr => Key::MetaRight,
        Key::MetaLeft => Key::Alt,
        Key::MetaRight => Key::AltGr,
        // Ctrl is left as-is.
        other => other.clone(),
    }
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

/// Async writer: forwards keyboard reports immediately and drains coalesced
/// mouse motion/wheel from the shared atomics once per [`FLUSH_INTERVAL`]
/// (plan §6.3).
async fn writer_loop(app: AppHandle, mut rx: UnboundedReceiver<InputMsg>) {
    // Resolve the managed state once, not on every report: the hot path runs at
    // the input rate (~1 kHz keys / 125 Hz mouse flushes), and `state()` is a
    // typemap lookup. `&state` deref-coerces to `&AppState` at each call site.
    let state = app.state::<AppState>();
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
                            reset_input_state(&state, &mut buttons);
                        }
                    }
                    InputMsg::Buttons(mask) => {
                        buttons = mask;
                        // Latency-sensitive: flush now, carrying pending motion.
                        if !flush_mouse(&state, buttons).await {
                            reset_input_state(&state, &mut buttons);
                        }
                    }
                    InputMsg::SetLock(active) => {
                        set_lock(&app, &state, active, &mut buttons).await;
                    }
                    InputMsg::GrabFailed => {
                        // Hook didn't install — undo the lock we optimistically
                        // entered (emits lock_state(false), releases capture).
                        set_lock(&app, &state, false, &mut buttons).await;
                    }
                    InputMsg::Shutdown => {
                        // Final safe state as the LAST writes (no queued report
                        // can re-press a key after these), then stop the writer.
                        let _ = write_keyboard(&state, &KEYBOARD_RELEASE_ALL).await;
                        let _ = write_mouse(&state, &MOUSE_RELEASE_ALL).await;
                        break;
                    }
                }
            }
            _ = interval.tick() => {
                // A sub-divisor wheel remainder alone isn't worth a packet; it
                // carries until it reaches a full tick or motion flushes.
                if motion_pending(&state) && !flush_mouse(&state, buttons).await {
                    reset_input_state(&state, &mut buttons);
                }
            }
        }
    }
}

/// App-exit teardown (plan §8/§9): force lock off so the grab produces no new
/// reports, then route a final release-all through the writer (so it is the
/// last write nothing can race) and wait for the writer to drain and stop.
/// Falls back to a direct release-all if the pipeline never started. The caller
/// bounds this with a timeout so a wedged link can't hang the quit.
pub async fn shutdown(state: &AppState) {
    state.input_shared.set_locked(false);
    if state.input_shared.pipeline_started() {
        state.input_shared.shutdown();
        let handle = { state.input_ctl.lock().await.take_writer() };
        match handle {
            Some(h) => {
                let _ = h.await;
            }
            // Writer already taken/stopped (e.g. a repeated ExitRequested) —
            // re-assert the safe state directly so it isn't skipped.
            None => state.conn.release_all().await,
        }
    } else {
        state.conn.release_all().await;
    }
}

/// Apply a lock-mode transition: set the flag, manage relative-mouse capture,
/// emit `lock_state`, and on exit send the safe all-up reports (plan §9).
async fn set_lock(app: &AppHandle, state: &AppState, active: bool, buttons: &mut u8) {
    let shared = &state.input_shared;

    // Clear hotkey modifier tracking on every transition. A Ctrl/Alt *release*
    // can be missed while the app briefly regains focus around a lock change
    // (WebView2 then swallows those key events), which would otherwise leave a
    // modifier stuck "down" and make the next session's exit hotkey misfire —
    // e.g. a lone Ctrl tripping it because Alt still reads as held. Entry is via
    // the UI button (no modifiers held) and after exit the physical keys no
    // longer matter, so resetting here is always safe.
    shared.ctrl_down.store(false, Ordering::SeqCst);
    shared.alt_down.store(false, Ordering::SeqCst);
    shared.hotkey_engaged.store(false, Ordering::SeqCst);

    if active {
        shared.lock_active.store(true, Ordering::SeqCst);
        // Freeze + decouple the cursor so movement is captured relatively
        // (only when we're actually grabbing the mouse channel).
        if shared.pass_mouse.load(Ordering::SeqCst) {
            platform::set_cursor_capture(true);
        }
        // Windows: drop our window out of the foreground so WebView2 doesn't
        // swallow the keyboard hook (no-op elsewhere). See the function.
        set_webview_foreground(app, true);
    } else {
        shared.lock_active.store(false, Ordering::SeqCst);
        platform::set_cursor_capture(false);
        set_webview_foreground(app, false);
        if let Ok(mut kb) = shared.keyboard.lock() {
            kb.reset();
        }
        clear_mouse_accumulators(shared);
        *buttons = 0;
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
        clear_mouse_accumulators(shared);
        // A zero-button, zero-motion flush clears any held button on the host.
        shared.send(InputMsg::Buttons(0));
    }
}

/// Zero the mouse button bitmask + coalesced motion/wheel accumulators.
fn clear_mouse_accumulators(shared: &InputShared) {
    shared.buttons.store(0, Ordering::SeqCst);
    shared.acc_dx.store(0, Ordering::SeqCst);
    shared.acc_dy.store(0, Ordering::SeqCst);
    shared.acc_wheel.store(0, Ordering::SeqCst);
}

/// Unconditionally release relative-mouse capture (unfreeze + show the cursor).
/// Idempotent and safe to call even if capture was never enabled. Used on app
/// exit and panic so the operator's cursor is never left frozen if the app dies
/// while locked (plan §8).
pub fn release_cursor_capture() {
    platform::set_cursor_capture(false);
}

/// Windows: keep the EmulStick window from being the keyboard-foreground window
/// while locked, so the global `WH_KEYBOARD_LL` hook actually receives keys.
///
/// Chromium/WebView2 swallows low-level keyboard-hook delivery while it holds
/// keyboard focus, so the grab callback never sees keystrokes typed into our own
/// window — breaking lock mode and the Ctrl+Alt exit hotkey whenever the app is
/// foreground, e.g. KVM screen mode (tauri-apps/tauri#13919; same Chromium
/// behaviour as CEF). The hook fires fine whenever any *other* window is
/// foreground, so on lock we mark the window non-activating (`WS_EX_NOACTIVATE`)
/// — so the operator's grab-click can't re-raise it — and hand the foreground to
/// the desktop shell, leaving our window visible so the KVM video keeps
/// rendering but no longer the keyboard-foreground window. On unlock we restore
/// activation and refocus so the UI is usable again. No-op off Windows; the app
/// needs no keyboard focus of its own (its UI is entirely mouse-driven).
///
/// If the window handle can't be resolved the keyboard hook silently won't
/// deliver, so we surface that as an `error` event rather than fail quietly.
#[cfg(target_os = "windows")]
pub fn set_webview_foreground(app: &AppHandle, locked: bool) {
    let Some(win) = app.get_webview_window("main") else {
        events::error(
            app,
            "lock_window_lookup_failed",
            "Could not find the app window to release keyboard focus; lock mode \
             and the Ctrl+Alt exit hotkey may not receive keys.",
        );
        return;
    };
    let Ok(hwnd) = win.hwnd() else {
        events::error(
            app,
            "lock_window_lookup_failed",
            "Could not resolve the app window handle to release keyboard focus; \
             lock mode and the Ctrl+Alt exit hotkey may not receive keys.",
        );
        return;
    };
    let hwnd = hwnd.0 as isize; // HWND isn't Send; carry it across as a pointer.
    let _ = app.run_on_main_thread(move || unsafe {
        use winapi::shared::windef::HWND;
        use winapi::um::winuser::{
            GetShellWindow, GetWindowLongPtrW, SetForegroundWindow, SetWindowLongPtrW, GWL_EXSTYLE,
            WS_EX_NOACTIVATE,
        };
        let h = hwnd as HWND;
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        if locked {
            // Non-activating: the operator's grab-click can't re-raise us. Then
            // hand the foreground to the desktop shell — flash-free, window stays
            // visible, but it's no longer the keyboard-foreground window.
            SetWindowLongPtrW(h, GWL_EXSTYLE, ex | WS_EX_NOACTIVATE as isize);
            let shell = GetShellWindow();
            if !shell.is_null() {
                SetForegroundWindow(shell);
            }
        } else {
            SetWindowLongPtrW(h, GWL_EXSTYLE, ex & !(WS_EX_NOACTIVATE as isize));
            SetForegroundWindow(h);
        }
    });
}

#[cfg(not(target_os = "windows"))]
pub fn set_webview_foreground(_app: &AppHandle, _locked: bool) {}

/// Whether the OS trusts this process to capture global input (macOS
/// Accessibility); always true elsewhere. See [`platform::accessibility_trusted`].
pub fn accessibility_trusted(prompt: bool) -> bool {
    platform::accessibility_trusted(prompt)
}

/// Emergency lock release for an **unexpected link drop** (plan §4.2/§8). Makes
/// the grab transparent and unfreezes the cursor *immediately*, synchronously,
/// without routing through the writer task — which may be mid-write to a dead
/// link. The operator must never stay trapped just because the dongle vanished.
///
/// No BLE release-all is sent (the link is already gone); the next successful
/// [`crate::ble::establish`] sends release-all anyway. Caller emits the
/// `lock_state(false)` IPC event.
pub fn emergency_unlock(shared: &InputShared) {
    shared.lock_active.store(false, Ordering::SeqCst);
    if let Ok(mut kb) = shared.keyboard.lock() {
        kb.reset();
    }
    clear_mouse_accumulators(shared);
    // Mirror `set_lock`'s hotkey-tracking reset so the two safe-state paths are
    // identical: a Ctrl/Alt held when the link dropped mustn't read as still
    // down on the next session.
    shared.ctrl_down.store(false, Ordering::SeqCst);
    shared.alt_down.store(false, Ordering::SeqCst);
    shared.hotkey_engaged.store(false, Ordering::SeqCst);
    platform::set_cursor_capture(false);
}

/// Whether there's mouse motion/wheel worth a packet (without consuming it).
fn motion_pending(state: &AppState) -> bool {
    let s = &state.input_shared;
    s.acc_dx.load(Ordering::SeqCst) != 0
        || s.acc_dy.load(Ordering::SeqCst) != 0
        || s.acc_wheel.load(Ordering::SeqCst).abs() >= SCROLL_DIVISOR
}

/// Drain the coalesced motion/wheel and flush it as F803 packets. Returns
/// `false` if any GATT write failed (so the caller can drop to a safe state,
/// plan §9).
async fn flush_mouse(state: &AppState, buttons: u8) -> bool {
    let shared = &state.input_shared;
    let dx = shared.acc_dx.swap(0, Ordering::SeqCst);
    let dy = shared.acc_dy.swap(0, Ordering::SeqCst);
    // Throttle scroll: one HID tick per SCROLL_DIVISOR accumulated line-deltas.
    // Clamp the per-flush tick count to the wheel field range and subtract only
    // what we send, so both the sub-divisor remainder *and* any excess beyond
    // ±127 carry to the next flush (a huge burst bleeds out, never truncated).
    let ticks = shared.acc_wheel.load(Ordering::SeqCst) / SCROLL_DIVISOR;
    let wheel = ticks.clamp(mouse::WHEEL_MIN, mouse::WHEEL_MAX);
    if wheel != 0 {
        shared.acc_wheel.fetch_sub(wheel * SCROLL_DIVISOR, Ordering::SeqCst);
    }
    let mut ok = true;
    for packet in mouse::split_reports(buttons, dx, dy, wheel) {
        ok &= write_mouse(state, &packet).await;
    }
    ok
}

/// Write a keyboard report; returns `false` if the GATT write failed.
async fn write_keyboard(state: &AppState, report: &[u8]) -> bool {
    state.conn.write_keyboard(report).await.is_ok()
}

/// Write a mouse report; returns `false` if the GATT write failed.
async fn write_mouse(state: &AppState, report: &[u8]) -> bool {
    state.conn.write_mouse(report).await.is_ok()
}

/// Drop to a safe logical state after a GATT write fails (plan §9): forget all
/// held keys/buttons and pending motion so no phantom press survives the
/// failure. The host itself is cleared by the release-all sent on the next
/// (re)connect ([`crate::ble::establish`]); the link is down now, so we can only
/// fix our own model here.
fn reset_input_state(state: &AppState, buttons: &mut u8) {
    let shared = &state.input_shared;
    if let Ok(mut kb) = shared.keyboard.lock() {
        kb.reset();
    }
    clear_mouse_accumulators(shared);
    *buttons = 0;
}

#[cfg(test)]
mod tests {
    use super::remap_for_target_os;
    use rdev::Key;

    // Host and target on the same OS ⇒ no remap (PC→PC and Mac→Mac alike).
    #[test]
    fn matching_os_is_identity() {
        for k in [Key::ControlLeft, Key::Alt, Key::MetaLeft, Key::AltGr, Key::KeyA] {
            assert_eq!(remap_for_target_os(&k, false, false), k); // PC host → PC target
            assert_eq!(remap_for_target_os(&k, true, true), k); //   Mac host → Mac target
        }
    }

    // Crossed host/target swaps Alt↔Win and leaves Ctrl alone. The swap is its
    // own inverse, so it holds both ways: PC→Mac and Mac→PC.
    #[test]
    fn crossed_os_swaps_alt_and_win_keeps_ctrl() {
        for &(target_mac, host_mac) in &[(true, false), (false, true)] {
            // Alt → Command(⌘), Win → Option(⌥)
            assert_eq!(remap_for_target_os(&Key::Alt, target_mac, host_mac), Key::MetaLeft);
            assert_eq!(remap_for_target_os(&Key::AltGr, target_mac, host_mac), Key::MetaRight);
            assert_eq!(remap_for_target_os(&Key::MetaLeft, target_mac, host_mac), Key::Alt);
            assert_eq!(remap_for_target_os(&Key::MetaRight, target_mac, host_mac), Key::AltGr);
            // Ctrl stays Control; regular keys untouched.
            assert_eq!(remap_for_target_os(&Key::ControlLeft, target_mac, host_mac), Key::ControlLeft);
            assert_eq!(remap_for_target_os(&Key::ControlRight, target_mac, host_mac), Key::ControlRight);
            assert_eq!(remap_for_target_os(&Key::KeyC, target_mac, host_mac), Key::KeyC);
        }
    }
}
