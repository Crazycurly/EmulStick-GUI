# EmulStick Desktop Protocol Notes

> [!NOTE]
> This is independent, implementation-focused documentation for EmulStick Desktop.
> It is **not** a copy of, replacement for, or authoritative version of the vendor's
> protocol specification. For the vendor documentation, see the official
> [EmulStick BLE protocol specification (v0.93)](https://www.emulstick.com/files/emulstick_ble_v0.93.pdf).

This file documents only the protocol details exercised by this repository and
validated by the encoders/tests under `src-tauri/src/protocol/`.

## BLE endpoints used by this application

| Purpose | UUID | Use |
| --- | --- | --- |
| Device Information Service | `0000180A-0000-1000-8000-00805f9b34fb` | Read device metadata |
| Custom service | `0000F800-0000-1000-8000-00805f9b34fb` | Parent service for input channels |
| Keyboard | `0000F801-0000-1000-8000-00805f9b34fb` | Write keyboard reports; receive LED notifications |
| Mouse | `0000F803-0000-1000-8000-00805f9b34fb` | Write mouse reports |

The application also reads firmware revision (`2A26`) and system ID (`2A23`)
from the standard Device Information Service when available.

## F801 keyboard report

The application emits an 8-byte keyboard report with no Report ID:

```text
[Modifiers, 0x00, Key1, Key2, Key3, Key4, Key5, Key6]
```

Modifier bits in byte 0:

| Bit | Modifier |
| ---: | --- |
| 0 | Left Ctrl |
| 1 | Left Shift |
| 2 | Left Alt |
| 3 | Left GUI / Win / Command |
| 4 | Right Ctrl |
| 5 | Right Shift |
| 6 | Right Alt |
| 7 | Right GUI / Win / Command |

Bytes 2–7 contain up to six USB HID usage IDs. The application re-sends the full
held-key state on every change. If more than six regular keys are held, the
encoder emits HID `ErrorRollOver` (`0x01`) in all six key slots.

The all-keys-up safety report is eight zero bytes.

### Worked example 1.1 — Right Win + Right Shift + S

These are the exact byte sequences asserted by the unit test in
`src-tauri/src/protocol/keyboard.rs`:

```text
Right Win down:             80 00 00 00 00 00 00 00
Right Shift down:           A0 00 00 00 00 00 00 00
S down (usage 0x16):        A0 00 16 00 00 00 00 00
S up:                       A0 00 00 00 00 00 00 00
Modifiers up:               00 00 00 00 00 00 00 00
```

### Worked example 1.2 — Left Shift + Up + Right

```text
Left Shift down:            02 00 00 00 00 00 00 00
Up down (usage 0x52):       02 00 52 00 00 00 00 00
Right down (usage 0x4F):    02 00 52 4F 00 00 00 00
Up up:                      02 00 4F 00 00 00 00 00
Right up:                   02 00 00 00 00 00 00 00
Shift up:                   00 00 00 00 00 00 00 00
```

## F803 mouse report

The application emits a 6-byte mouse report with no Report ID:

```text
[Buttons, X_low, X_high, Y_low, Y_high, Wheel]
```

- `Buttons` uses bits 0–4 for left, right, middle, button 4, and button 5.
- X and Y are signed 16-bit little-endian relative deltas.
- A single packet is limited by the implementation to `-2047..+2047` per axis.
- Wheel is a signed 8-bit value limited to `-127..+127`.
- Larger accumulated pointer deltas are split across multiple reports rather
  than truncated.
- The all-buttons-up/no-motion safety report is six zero bytes.

### Worked example 2 — mouse reports

These values are asserted by `src-tauri/src/protocol/mouse.rs`:

```text
Move +6, +12:               00 06 00 0C 00 00
Move -2, +3:                00 FE FF 03 00 00
Left button down:           01 00 00 00 00 00
All buttons up:             00 00 00 00 00 00
```

## Keyboard LED notifications

The F801 characteristic can provide the standard one-byte keyboard LED state.
EmulStick Desktop subscribes to the notification and uses it for lock-key state
such as Num Lock, Caps Lock, and Scroll Lock.

## Safety behavior

On disconnect, lock-mode exit, relevant write failure, or other teardown paths,
the application sends release-all state where possible so the target is not
left with a logically held key or mouse button.

## Scope

This document intentionally omits protocol channels and capabilities that the
current desktop application does not implement. Consult the vendor's official
protocol specification for the complete device protocol and authoritative
compatibility information.
