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
