//! Platform cursor warping for relative-mouse capture.
//!
//! Uses APIs that move the pointer **without** generating an input event, so
//! the recenter doesn't feed back into our own grab hook:
//! * macOS — `CGWarpMouseCursorPosition` (CoreGraphics).
//! * Windows — `SetCursorPos` (user32).

#[cfg(target_os = "macos")]
pub fn warp_cursor(x: f64, y: f64) {
    #[repr(C)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGWarpMouseCursorPosition(new_cursor_position: CGPoint) -> i32;
    }
    unsafe {
        CGWarpMouseCursorPosition(CGPoint { x, y });
    }
}

#[cfg(target_os = "windows")]
pub fn warp_cursor(x: f64, y: f64) {
    #[link(name = "user32")]
    extern "system" {
        fn SetCursorPos(x: i32, y: i32) -> i32;
    }
    unsafe {
        SetCursorPos(x as i32, y as i32);
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn warp_cursor(_x: f64, _y: f64) {}
