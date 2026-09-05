# Vendored `rdev` 0.5.3 — EmulStick patches

Unmodified copy of [`rdev` 0.5.3](https://crates.io/crates/rdev/0.5.3) except for
two macOS-only changes (Patches 1–2) and two Windows changes (Patches 3–4).

---

## Patch 4 — Windows: skip `Event::name` resolution on the grab thread (`src/windows/grab.rs`)

### The change

`raw_callback` no longer calls `Keyboard::get_name(lpdata)` for KeyPress events;
it sets `Event { name: None, .. }`. The `EventType`/`KEYBOARD` imports it needed
are dropped with it.

### Why

Upstream resolves `Event::name` on **every KeyPress** from inside the
`WH_KEYBOARD_LL` hook. `get_name` → `set_global_state` calls
`AttachThreadInput(grab_thread, foreground_thread, TRUE)` followed by
`GetKeyboardState` + `ToUnicodeEx` against the foreground window's thread. Doing
that synchronously inside a low-level keyboard hook routinely exceeds Windows'
`LowLevelHooksTimeout` (~300 ms), so the OS **silently stops dispatching the
keyboard hook**. Symptom: keystrokes bypass `grab` entirely (the host keeps
typing, reserved combos and the Ctrl+Alt exit hotkey never fire) while the
**mouse** hook — which never computes a name (`raw_callback`'s `_ => None`) —
keeps working. That keyboard-only failure is the giveaway.

EmulStick maps the `rdev::Key` enum (`Event::event_type`) via `protocol::keymap`
and **never reads `Event::name`**, so dropping the name computation is
behaviourally invisible to us and keeps the keyboard hook alive. This is the
exact Windows analogue of Patch 1 (macOS skips the Text Services Manager name
lookup on the grab thread for the same reason).

### Upgrading

If bumping `rdev`, re-apply (or drop the vendor entirely if a future release
runs name resolution off the hook callback / lets you disable it).

---

## Patch 3 — Windows: map Right-Win / VK 92 (`src/windows/keycodes.rs`)

Upstream's `decl_keycodes!` table has `MetaLeft` (VK 91) but omits the Right
Windows key (VK 92), so it decodes to `Key::Unknown(92)` and the keymap drops
it. Added `MetaRight, 92` so the right `Win` key forwards as Right-GUI, matching
the left key and `protocol::keymap`'s existing `Key::MetaRight` mapping. The
reversibility test still holds (92 was previously unmapped). Linux/macOS already
map their right-meta key, so only the Windows table changes.

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
