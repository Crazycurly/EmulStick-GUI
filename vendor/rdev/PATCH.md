# Vendored `rdev` 0.5.3 — EmulStick patch

This is an unmodified copy of [`rdev` 0.5.3](https://crates.io/crates/rdev/0.5.3)
**except** for a single macOS-only change in `src/macos/keyboard.rs`.

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
