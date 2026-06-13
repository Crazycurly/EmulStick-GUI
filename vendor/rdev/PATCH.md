# Vendored `rdev` 0.5.3 — EmulStick patches

Unmodified copy of [`rdev` 0.5.3](https://crates.io/crates/rdev/0.5.3) except for
two macOS-only changes (Windows/Linux paths untouched).

---

## Patch 2 — relative mouse deltas + line-based scroll (`src/macos/common.rs`)

Two mouse changes in `convert`, both using only `CGEventGetIntegerValueField`
(thread-safe), unlike the NSEvent bridge:

* `EventType::MouseMove { x, y }` now carries the **relative HID delta**
  (`MOUSE_EVENT_DELTA_X/Y`) instead of the absolute cursor position, and
  `*MouseDragged` (movement with a button held) is handled too. This lets the
  app capture motion while the cursor is frozen via
  `CGAssociateMouseAndMouseCursorPosition(false)` — without it, a consumed
  mouse-move doesn't actually stop the OS cursor, so it drifts into screen
  edges where position-based deltas die.
* `EventType::Wheel` now reads the **line** delta (`SCROLL_WHEEL_EVENT_DELTA_AXIS_1/2`)
  instead of the **point** (pixel) delta. Pixel deltas are huge for trackpads
  and made host scrolling race; line deltas are the natural HID wheel unit.

---

## Patch 1 — skip Text Services Manager on the grab thread

## The change

`Keyboard::create_string_for_key` is patched to return `None` instead of calling
`string_from_code` (which uses `TISCopyCurrentKeyboardInputSource` /
`UCKeyTranslate`).

## Why

On macOS, `rdev::grab`'s event-tap callback calls `convert()`, which computes
`Event::name` for every key event via the **Text Services Manager**. Those TSM
APIs assert they run on the **main thread** (`dispatch_assert_queue`). EmulStick
runs `grab` on a dedicated background thread (Tauri owns the main thread), so the
assertion fires and the process dies with **SIGTRAP** on the first keystroke.

Confirmed via crash report:

```
rdev-grab thread — dispatch_assert_queue_fail
  HIToolbox  TSMGetInputSourceProperty
  rdev::macos::keyboard::Keyboard::create_string_for_key
  rdev::macos::common::convert
  rdev::macos::grab::raw_callback
```

EmulStick never reads `Event::name` — the keymap maps `Event::event_type` (the
`rdev::Key` enum, built purely from the keycode with no TSM call). So dropping
the name computation is behaviourally invisible to us and keeps the grab thread
alive. Only macOS code is touched; the Windows path is identical to upstream.

## Upgrading

If bumping `rdev`, re-apply this one change (or drop the vendor entirely if a
future release adds a way to disable name resolution / runs name lookup
off the tap callback).
