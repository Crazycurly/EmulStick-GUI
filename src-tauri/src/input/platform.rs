//! Platform cursor control for relative-mouse capture.
//!
//! On lock + mouse passthrough we decouple the OS cursor from the physical
//! mouse so it stays put while we still receive movement deltas (rdev reports
//! `MOUSE_EVENT_DELTA_X/Y` — see `vendor/rdev/PATCH.md`). Consuming move events
//! alone does **not** freeze the macOS cursor.

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

#[cfg(target_os = "windows")]
pub fn set_cursor_capture(_capture: bool) {
    // TODO(M2/Windows): clip the cursor to the window and use RAWINPUT deltas;
    // rdev on Windows still reports absolute positions. Not yet implemented.
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn set_cursor_capture(_capture: bool) {}
