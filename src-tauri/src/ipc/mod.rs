//! Tauri IPC surface: low-frequency commands (frontend → backend) and events
//! (backend → frontend). Per plan §3/§7 this channel deliberately carries no
//! per-frame cursor deltas or keystrokes — those stay in the Rust data plane.

pub mod commands;
pub mod events;

use serde::{Deserialize, Serialize};

/// A device surfaced during scanning. `id` is the opaque, machine-local
/// `Peripheral::id()` string (plan §4.1) — treat as a key, not a MAC address.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredDevice {
    pub id: String,
    pub name: Option<String>,
    pub rssi: Option<i16>,
}

/// Connection lifecycle, broadcast to the UI via the `connection_state` event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConnectionState {
    Scanning,
    Connecting,
    Connected,
    Disconnected,
    Reconnecting,
}

/// Device Information Service readout shown after connect (plan §4.1).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub firmware: Option<String>,
    pub model: Option<String>,
    pub manufacturer: Option<String>,
    /// Hex-encoded 8-byte System ID (`2A23`).
    pub system_id: Option<String>,
}

/// Keyboard LED state decoded from the F801 1-byte LED report.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct LedReport {
    pub num: bool,
    pub caps: bool,
    pub scroll: bool,
}

impl LedReport {
    /// Decode the 1-byte LED report (bit0 Num, bit1 Caps, bit2 Scroll).
    pub fn from_byte(b: u8) -> Self {
        Self {
            num: b & 0x01 != 0,
            caps: b & 0x02 != 0,
            scroll: b & 0x04 != 0,
        }
    }
}
