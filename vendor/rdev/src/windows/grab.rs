use crate::rdev::{Event, GrabError};
use crate::windows::common::{convert, set_key_hook, set_mouse_hook, HookError, HOOK};
use std::ptr::null_mut;
use std::time::SystemTime;
use winapi::um::winuser::{CallNextHookEx, GetMessageA, HC_ACTION};

static mut GLOBAL_CALLBACK: Option<Box<dyn FnMut(Event) -> Option<Event>>> = None;

unsafe extern "system" fn raw_callback(code: i32, param: usize, lpdata: isize) -> isize {
    if code == HC_ACTION {
        let opt = convert(param, lpdata);
        if let Some(event_type) = opt {
            // PATCH 4 (EmulStick) — do NOT resolve `Event::name` here. Upstream
            // calls `Keyboard::get_name` on every KeyPress, which runs
            // `AttachThreadInput` + `GetKeyboardState` + `ToUnicodeEx` against
            // the foreground thread from *inside* the WH_KEYBOARD_LL callback.
            // That work routinely exceeds `LowLevelHooksTimeout` (~300 ms), so
            // Windows silently stops calling the keyboard hook — keystrokes then
            // bypass the grab (host keeps typing, the Ctrl+Alt exit hotkey never
            // fires) while the mouse hook, which never computes a name, keeps
            // working. EmulStick maps `Key` (event_type) only and never reads
            // `name`, so dropping it is behaviourally invisible. This is the
            // Windows twin of the macOS TSM patch — see vendor/rdev/PATCH.md.
            let event = Event {
                event_type,
                time: SystemTime::now(),
                name: None,
            };
            if let Some(callback) = &mut GLOBAL_CALLBACK {
                if callback(event).is_none() {
                    // https://stackoverflow.com/questions/42756284/blocking-windows-mouse-click-using-setwindowshookex
                    // https://android.developreference.com/article/14560004/Blocking+windows+mouse+click+using+SetWindowsHookEx()
                    // https://cboard.cprogramming.com/windows-programming/99678-setwindowshookex-wm_keyboard_ll.html
                    // let _result = CallNextHookEx(HOOK, code, param, lpdata);
                    return 1;
                }
            }
        }
    }
    CallNextHookEx(HOOK, code, param, lpdata)
}
impl From<HookError> for GrabError {
    fn from(error: HookError) -> Self {
        match error {
            HookError::Mouse(code) => GrabError::MouseHookError(code),
            HookError::Key(code) => GrabError::KeyHookError(code),
        }
    }
}

pub fn grab<T>(callback: T) -> Result<(), GrabError>
where
    T: FnMut(Event) -> Option<Event> + 'static,
{
    unsafe {
        GLOBAL_CALLBACK = Some(Box::new(callback));
        set_key_hook(raw_callback)?;
        set_mouse_hook(raw_callback)?;

        GetMessageA(null_mut(), null_mut(), 0, 0);
    }
    Ok(())
}
