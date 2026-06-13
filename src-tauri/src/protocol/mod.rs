//! EmulStick HID report encoders and the OS-key → HID-usage keymap.
//!
//! This module is hardware-independent and fully unit-tested against the
//! worked examples in `docs/protocol.md`; it is the validation cornerstone of
//! milestone M1. Nothing here performs I/O.

pub mod keyboard;
pub mod keymap;
pub mod mouse;

pub use keyboard::{KeyboardState, KEYBOARD_RELEASE_ALL, KEYBOARD_REPORT_LEN};
pub use keymap::{map_key, Mapping};
pub use mouse::{encode_report, split_reports, MOUSE_RELEASE_ALL, MOUSE_REPORT_LEN};

/// Normative BLE UUIDs (16-bit assigned numbers expanded to the Bluetooth base
/// UUID). All EmulStick payloads target the Custom Service `0000F800-…`.
pub mod uuids {
    use uuid::Uuid;

    /// Build a 128-bit UUID from a 16-bit assigned number using the Bluetooth
    /// base UUID `0000xxxx-0000-1000-8000-00805f9b34fb`.
    pub const fn from_u16(short: u16) -> Uuid {
        Uuid::from_u128(
            0x0000_0000_0000_1000_8000_00805f9b34fb_u128 | ((short as u128) << 96),
        )
    }

    /// Device Information Service (`180A`).
    pub const DEVICE_INFO_SERVICE: Uuid = from_u16(0x180A);
    /// System ID characteristic (`2A23`).
    pub const SYSTEM_ID: Uuid = from_u16(0x2A23);
    /// Firmware Revision characteristic (`2A26`).
    pub const FIRMWARE_REVISION: Uuid = from_u16(0x2A26);
    /// Model Number characteristic (`2A24`).
    pub const MODEL_NUMBER: Uuid = from_u16(0x2A24);
    /// Manufacturer Name characteristic (`2A29`).
    pub const MANUFACTURER_NAME: Uuid = from_u16(0x2A29);

    /// Custom Service (`F800`) — the EmulStick data channels.
    pub const CUSTOM_SERVICE: Uuid = from_u16(0xF800);
    /// Keyboard channel (`F801`), 8-byte reports, write-without-response.
    pub const KEYBOARD: Uuid = from_u16(0xF801);
    /// Mouse channel (`F803`), 6-byte reports, write-without-response.
    pub const MOUSE: Uuid = from_u16(0xF803);

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn short_uuids_expand_to_bluetooth_base() {
            assert_eq!(
                KEYBOARD.to_string(),
                "0000f801-0000-1000-8000-00805f9b34fb"
            );
            assert_eq!(MOUSE.to_string(), "0000f803-0000-1000-8000-00805f9b34fb");
            assert_eq!(
                CUSTOM_SERVICE.to_string(),
                "0000f800-0000-1000-8000-00805f9b34fb"
            );
            assert_eq!(
                FIRMWARE_REVISION.to_string(),
                "00002a26-0000-1000-8000-00805f9b34fb"
            );
        }
    }
}
