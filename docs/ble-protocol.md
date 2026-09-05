# BLE Transport Notes

> [!NOTE]
> This document describes how **EmulStick Desktop** uses BLE. It is independent
> implementation documentation, not a mirror of the vendor specification. For
> the authoritative device protocol, see the official
> [EmulStick BLE protocol specification (v0.93)](https://www.emulstick.com/files/emulstick_ble_v0.93.pdf).

## Scope

The desktop application currently uses the BLE channels needed for keyboard and
mouse forwarding plus standard device-information reads. Other vendor-defined
channels are intentionally outside the scope of this document.

## Services and characteristics

| Purpose | UUID | Application behavior |
| --- | --- | --- |
| Device Information Service | `0000180A-0000-1000-8000-00805f9b34fb` | Read metadata after connection |
| Custom service | `0000F800-0000-1000-8000-00805f9b34fb` | Locate the input characteristics |
| Keyboard | `0000F801-0000-1000-8000-00805f9b34fb` | Write-without-response; subscribe to LED notifications |
| Mouse | `0000F803-0000-1000-8000-00805f9b34fb` | Write-without-response |

The implementation reads firmware revision (`2A26`) and system ID (`2A23`) from
the Device Information Service when those characteristics are available.

## Connection flow

1. Scan for a compatible BLE peripheral.
2. Connect through the platform BLE stack (`btleplug`).
3. Discover the custom F800 service and F801/F803 characteristics.
4. Read available Device Information Service metadata.
5. Subscribe to F801 keyboard-LED notifications.
6. Forward keyboard and mouse state through write-without-response operations.
7. On disconnect or teardown, clear local forwarding state and attempt safe
   release-all behavior where communication is still possible.

## Reports emitted by this application

The byte layouts that are authoritative for this repository are implemented in
`src-tauri/src/protocol/` and summarized in [`protocol.md`](protocol.md):

- **F801 keyboard:** 8 bytes — `[Modifiers, 0x00, Key1..Key6]`
- **F803 mouse:** 6 bytes — `[Buttons, X_low, X_high, Y_low, Y_high, Wheel]`

X/Y mouse deltas are signed little-endian values and the application splits
large accumulated movement across multiple reports rather than silently
truncating it.

## Platform identity note

A BLE peripheral identifier should be treated as an opaque, machine-local
identifier. Platform BLE APIs do not guarantee that the identifier exposed to
the application is a portable hardware MAC address across operating systems or
machines.

## Compatibility

If behavior in these notes differs from the vendor's current official
specification, the vendor specification should be treated as authoritative for
the hardware. The source code and tests in this repository remain authoritative
for what this particular application actually emits.
