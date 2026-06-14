//! Platform cursor control for relative-mouse capture.
//!
//! On lock + mouse passthrough we decouple the OS cursor from the physical
//! mouse so it stays put while we still receive movement deltas. Consuming move
//! events alone does **not** freeze the cursor on either OS.
//!
//! The two platforms get their relative deltas differently:
//! * **macOS** — the vendored rdev patch makes `MouseMove { x, y }` carry the
//!   HID delta directly (`MOUSE_EVENT_DELTA_X/Y`, see `vendor/rdev/PATCH.md`),
//!   and `CGAssociateMouseAndMouseCursorPosition(false)` freezes the cursor.
//! * **Windows** — rdev's low-level hook reports the **absolute** cursor point
//!   (`vendor/rdev/src/windows/common.rs`). So [`relative_delta`] turns that
//!   absolute point into a delta against a fixed screen-centre anchor and warps
//!   the cursor back to centre after each move (classic "mouselook" recenter).
//!   This keeps deltas finite and the cursor pinned without a vendored patch.

/// Enable (`true`) or release (`false`) relative-mouse capture: freeze the
/// system cursor and decouple it from the physical mouse.
#[cfg(target_os = "macos")]
pub fn set_cursor_capture(capture: bool) {
    #[repr(C)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGAssociateMouseAndMouseCursorPosition(connected: i32) -> i32;
        fn CGWarpMouseCursorPosition(new_cursor_position: CGPoint) -> i32;
        fn CGMainDisplayID() -> u32;
        fn CGDisplayPixelsWide(display: u32) -> usize;
        fn CGDisplayPixelsHigh(display: u32) -> usize;
        fn CGDisplayHideCursor(display: u32) -> i32;
        fn CGDisplayShowCursor(display: u32) -> i32;
    }
    unsafe {
        let display = CGMainDisplayID();
        if capture {
            // Centre the cursor first so it isn't frozen against a screen edge,
            // then decouple it from the mouse and hide it.
            let x = CGDisplayPixelsWide(display) as f64 / 2.0;
            let y = CGDisplayPixelsHigh(display) as f64 / 2.0;
            CGWarpMouseCursorPosition(CGPoint { x, y });
            CGAssociateMouseAndMouseCursorPosition(0);
            CGDisplayHideCursor(display);
        } else {
            CGAssociateMouseAndMouseCursorPosition(1);
            CGDisplayShowCursor(display);
        }
    }
}

/// Convert a `MouseMove` event's coordinates into a relative HID delta.
///
/// On macOS the vendored rdev patch already delivers deltas, so this is the
/// identity. (Windows overrides this — see the `target_os = "windows"` variant.)
#[cfg(not(target_os = "windows"))]
pub fn relative_delta(x: f64, y: f64) -> (i32, i32) {
    (x as i32, y as i32)
}

#[cfg(target_os = "windows")]
mod win {
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    use winapi::shared::minwindef::{FALSE, TRUE};
    use winapi::um::winuser::{GetSystemMetrics, SetCursorPos, ShowCursor, SM_CXSCREEN, SM_CYSCREEN};

    /// Whether relative-mouse capture is active. Gates [`super::relative_delta`]
    /// so absolute points are only turned into deltas while we're capturing, and
    /// guards `ShowCursor` so its display counter stays balanced (one hide per
    /// enable, one show per disable) across the many idempotent disable calls.
    static CAPTURED: AtomicBool = AtomicBool::new(false);
    /// Screen-centre anchor we warp the cursor back to after each move.
    static CENTER_X: AtomicI32 = AtomicI32::new(0);
    static CENTER_Y: AtomicI32 = AtomicI32::new(0);

    pub fn set_cursor_capture(capture: bool) {
        unsafe {
            if capture {
                // Idempotent: a second enable (e.g. a passthrough refresh while
                // already locked) must not hide the cursor twice.
                if CAPTURED.swap(true, Ordering::SeqCst) {
                    return;
                }
                // Anchor at the primary monitor's centre. Multi-monitor / the
                // active window's monitor is a future refinement (plan §Windows).
                let cx = GetSystemMetrics(SM_CXSCREEN) / 2;
                let cy = GetSystemMetrics(SM_CYSCREEN) / 2;
                CENTER_X.store(cx, Ordering::SeqCst);
                CENTER_Y.store(cy, Ordering::SeqCst);
                SetCursorPos(cx, cy);
                // Best-effort hide. ShowCursor only reliably hides over our own
                // process's windows; with the cursor pinned at centre and all
                // moves consumed this is cosmetic, so the limitation is benign.
                ShowCursor(FALSE);
            } else {
                // Idempotent: disable runs from lock-exit, passthrough changes,
                // emergency unlock, and the panic/exit hooks — only undo once.
                if !CAPTURED.swap(false, Ordering::SeqCst) {
                    return;
                }
                ShowCursor(TRUE);
            }
        }
    }

    /// Turn rdev's **absolute** cursor point into a delta against the centre
    /// anchor, then warp the cursor back to centre so the next event's delta is
    /// measured from the same origin (and the cursor stays pinned). Returns
    /// `(0, 0)` when not capturing.
    ///
    /// Per-event recentre keeps deltas finite without `ClipCursor`: since every
    /// move starts from centre, a single low-level-hook event (one mouse HID
    /// report) can't travel far enough to saturate at a screen edge. If fast
    /// flicks ever stick at an edge on real hardware, the fallback is true Raw
    /// Input (`WM_INPUT` `RAWMOUSE.lLastX/lLastY`) via a message-only window.
    ///
    /// The warp itself generates a synthetic move at exactly `(cx, cy)`, which
    /// re-enters here as a `(0, 0)` delta and is skipped — no feedback loop.
    pub fn relative_delta(x: f64, y: f64) -> (i32, i32) {
        if !CAPTURED.load(Ordering::SeqCst) {
            return (0, 0);
        }
        let cx = CENTER_X.load(Ordering::SeqCst);
        let cy = CENTER_Y.load(Ordering::SeqCst);
        let dx = x as i32 - cx;
        let dy = y as i32 - cy;
        if dx != 0 || dy != 0 {
            unsafe {
                SetCursorPos(cx, cy);
            }
        }
        (dx, dy)
    }
}

#[cfg(target_os = "windows")]
pub use win::{relative_delta, set_cursor_capture};

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn set_cursor_capture(_capture: bool) {}

/// Whether this process is trusted for Accessibility — i.e. allowed to install
/// a `CGEventTap`/`rdev::grab` (plan §5). Without it the global input hook
/// silently fails, so the UI checks this before entering lock and guides the
/// operator to grant it.
///
/// When `prompt` is true and the process is not yet trusted, macOS shows its
/// "<app> would like to control this computer" dialog and adds the app to the
/// Accessibility list — the onboarding entry point.
#[cfg(target_os = "macos")]
pub fn accessibility_trusted(prompt: bool) -> bool {
    use std::ffi::c_void;
    use std::ptr;

    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFDictionaryRef = *const c_void;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFBooleanTrue: CFTypeRef;
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
        fn CFDictionaryCreate(
            allocator: CFTypeRef,
            keys: *const CFTypeRef,
            values: *const CFTypeRef,
            num_values: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> CFDictionaryRef;
        fn CFRelease(cf: CFTypeRef);
    }
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        static kAXTrustedCheckOptionPrompt: CFStringRef;
        // Returns a `Boolean` (unsigned char); `NULL` options == AXIsProcessTrusted().
        fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> u8;
    }

    unsafe {
        let options: CFDictionaryRef = if prompt {
            let key: CFTypeRef = kAXTrustedCheckOptionPrompt;
            let value: CFTypeRef = kCFBooleanTrue;
            CFDictionaryCreate(
                ptr::null(),
                &key,
                &value,
                1,
                &kCFTypeDictionaryKeyCallBacks,
                &kCFTypeDictionaryValueCallBacks,
            )
        } else {
            ptr::null()
        };
        let trusted = AXIsProcessTrustedWithOptions(options) != 0;
        if !options.is_null() {
            CFRelease(options);
        }
        trusted
    }
}

/// Non-macOS platforms have no equivalent gate (Windows low-level hooks need no
/// entitlement), so the process is always "trusted" to capture input.
#[cfg(not(target_os = "macos"))]
pub fn accessibility_trusted(_prompt: bool) -> bool {
    true
}
