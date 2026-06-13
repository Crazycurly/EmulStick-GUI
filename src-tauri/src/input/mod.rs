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
use crate::state::AppState;

/// Mouse flush cadence. Decouples ~1000 Hz input from the BLE pipe; should sit
/// near the negotiated BLE connection interval (plan §6.3 / §10).
const FLUSH_INTERVAL: Duration = Duration::from_millis(8);

/// Recenter the cursor once it strays this far (px) from screen centre, so the
/// operator can't run into a screen edge and lose relative tracking.
const RECENTER_RADIUS: f64 = 200.0;

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
    /// Last observed absolute cursor position, for relative-delta computation.
    last_pos: Mutex<Option<(f64, f64)>>,
    /// Screen centre, recomputed on each lock-enter.
    center: Mutex<(f64, f64)>,

    /// Ctrl/Alt held flags + edge latch for the lock hotkey.
    ctrl_down: AtomicBool,
    alt_down: AtomicBool,
    hotkey_engaged: AtomicBool,

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
            last_pos: Mutex::new(None),
            center: Mutex::new((0.0, 0.0)),
            ctrl_down: AtomicBool::new(false),
            alt_down: AtomicBool::new(false),
            hotkey_engaged: AtomicBool::new(false),
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

    /// Install the grab hook + writer task once. Subsequent calls are no-ops.
    /// On macOS the grab fails until Accessibility is granted; that surfaces as
    /// an `error` event from the grab thread (plan §5).
    pub fn ensure_started(&mut self, app: &AppHandle) {
        if self.started {
            return;
        }
        self.started = true;

        let (tx, rx) = unbounded_channel::<InputMsg>();
        let _ = self.shared.tx.set(tx);

        // Async writer task: owns the BLE-facing side and §6.3 coalescing.
        let writer = {
            let app = app.clone();
            tauri::async_runtime::spawn(writer_loop(app, rx))
        };

        // Dedicated OS thread for the (blocking) grab event loop.
        let grab_thread = {
            let shared = self.shared.clone();
            let app = app.clone();
            std::thread::Builder::new()
                .name("rdev-grab".into())
                .spawn(move || {
                    let cb_shared = shared.clone();
                    tracing::info!("installing global input grab hook");
                    let result = rdev::grab(move |event| handle_event(&cb_shared, event));
                    if let Err(e) = result {
                        tracing::error!(?e, "rdev::grab failed (Accessibility permission?)");
                        events::error(
                            &app,
                            "input_grab_failed",
                            format!(
                                "Could not install the global input hook ({e:?}). On macOS, \
                                 enable this app under System Settings → Privacy & Security → \
                                 Accessibility, then relaunch."
                            ),
                        );
                    }
                })
                .ok()
        };

        self._writer = Some(writer);
        self._grab_thread = grab_thread;
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
            // Track position even when not forwarding, so the first locked
            // delta isn't a huge jump from a stale origin.
            if !shared.is_locked() || !shared.pass_mouse.load(Ordering::SeqCst) {
                if let Ok(mut last) = shared.last_pos.lock() {
                    *last = Some((*x, *y));
                }
                return Some(event);
            }

            let center = shared.center.lock().map(|c| *c).unwrap_or((0.0, 0.0));
            if let Ok(mut last) = shared.last_pos.lock() {
                let (dx, dy) = match *last {
                    Some((px, py)) => ((x - px) as i32, (y - py) as i32),
                    None => (0, 0),
                };
                if dx != 0 || dy != 0 {
                    shared.send(InputMsg::Move { dx, dy });
                }
                // Recenter near edges to keep relative tracking unbounded.
                if (x - center.0).hypot(y - center.1) > RECENTER_RADIUS {
                    platform::warp_cursor(center.0, center.1);
                    *last = Some(center);
                } else {
                    *last = Some((*x, *y));
                }
            }
            None
        }

        EventType::Wheel { delta_x: _, delta_y } => {
            if !shared.is_locked() || !shared.pass_mouse.load(Ordering::SeqCst) {
                return Some(event);
            }
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
        Key::Alt | Key::AltGr => shared.alt_down.store(pressed, Ordering::SeqCst),
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
    let mut acc_dx: i32 = 0;
    let mut acc_dy: i32 = 0;
    let mut acc_wheel: i32 = 0;
    let mut buttons: u8 = 0;
    let mut interval = tokio::time::interval(FLUSH_INTERVAL);

    loop {
        tokio::select! {
            msg = rx.recv() => {
                let Some(msg) = msg else { break };
                match msg {
                    InputMsg::Keyboard(report) => {
                        write_keyboard(&app, &report).await;
                    }
                    InputMsg::Buttons(mask) => {
                        buttons = mask;
                        // Latency-sensitive: flush now, carrying pending motion.
                        flush_mouse(&app, buttons, &mut acc_dx, &mut acc_dy, &mut acc_wheel).await;
                    }
                    InputMsg::Move { dx, dy } => {
                        acc_dx += dx;
                        acc_dy += dy;
                    }
                    InputMsg::Wheel(w) => {
                        acc_wheel += w;
                    }
                    InputMsg::SetLock(active) => {
                        set_lock(&app, active, &mut buttons, &mut acc_dx, &mut acc_dy, &mut acc_wheel).await;
                    }
                }
            }
            _ = interval.tick() => {
                if acc_dx != 0 || acc_dy != 0 || acc_wheel != 0 {
                    flush_mouse(&app, buttons, &mut acc_dx, &mut acc_dy, &mut acc_wheel).await;
                }
            }
        }
    }
}

/// Apply a lock-mode transition: set the flag, (re)compute screen centre on
/// entry, emit `lock_state`, and on exit send the safe all-up reports (plan §9).
async fn set_lock(
    app: &AppHandle,
    active: bool,
    buttons: &mut u8,
    acc_dx: &mut i32,
    acc_dy: &mut i32,
    acc_wheel: &mut i32,
) {
    let state = app.state::<AppState>();
    let shared = &state.input_shared;

    if active {
        if let Ok((w, h)) = rdev::display_size() {
            if let Ok(mut c) = shared.center.lock() {
                *c = (w as f64 / 2.0, h as f64 / 2.0);
            }
        }
        if let Ok(mut last) = shared.last_pos.lock() {
            *last = None;
        }
        // Set the flag last so the callback never acts before centre is ready.
        shared.lock_active.store(true, Ordering::SeqCst);
    } else {
        shared.lock_active.store(false, Ordering::SeqCst);
        if let Ok(mut kb) = shared.keyboard.lock() {
            kb.reset();
        }
        shared.buttons.store(0, Ordering::SeqCst);
        *buttons = 0;
        *acc_dx = 0;
        *acc_dy = 0;
        *acc_wheel = 0;
        write_keyboard(app, &KEYBOARD_RELEASE_ALL).await;
        write_mouse(app, &MOUSE_RELEASE_ALL).await;
    }

    events::lock_state(app, active);
}

async fn flush_mouse(
    app: &AppHandle,
    buttons: u8,
    acc_dx: &mut i32,
    acc_dy: &mut i32,
    acc_wheel: &mut i32,
) {
    for packet in mouse::split_reports(buttons, *acc_dx, *acc_dy, *acc_wheel) {
        write_mouse(app, &packet).await;
    }
    *acc_dx = 0;
    *acc_dy = 0;
    *acc_wheel = 0;
}

async fn write_keyboard(app: &AppHandle, report: &[u8]) {
    let _ = app
        .state::<AppState>()
        .ble
        .lock()
        .await
        .write_keyboard(report)
        .await;
}

async fn write_mouse(app: &AppHandle, report: &[u8]) {
    let _ = app
        .state::<AppState>()
        .ble
        .lock()
        .await
        .write_mouse(report)
        .await;
}
