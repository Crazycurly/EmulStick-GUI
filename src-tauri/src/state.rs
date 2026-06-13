//! Shared application state: passthrough flags, lock state, and the BLE
//! manager handle. Held behind `tauri::State` and shared across commands.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::ble::BleManager;

/// The three independent passthrough flags (plan §4.3). When a flag is off the
/// corresponding channel is *not* grabbed and behaves normally on the operator
/// OS; when on it is grabbed and forwarded to the host.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PassthroughFlags {
    pub keyboard: bool,
    pub mouse: bool,
    pub video: bool,
}

impl Default for PassthroughFlags {
    fn default() -> Self {
        // Conservative default: forward nothing until the operator opts in.
        Self {
            keyboard: false,
            mouse: false,
            video: false,
        }
    }
}

/// Root application state managed by Tauri.
///
/// `ble` is an async `Mutex` because every BLE operation is `async` and must
/// not block the UI thread. `lock_active` is a lightweight atomic so the
/// grab thread can read it without taking a lock.
pub struct AppState {
    pub ble: Mutex<BleManager>,
    pub passthrough: Mutex<PassthroughFlags>,
    pub lock_active: AtomicBool,
    /// Opaque, machine-local `Peripheral::id()` of the last device we connected
    /// to, for auto-reconnect. **Not** portable across machines (plan §4.1).
    pub last_device_id: Mutex<Option<String>>,
}

impl AppState {
    pub fn new(ble: BleManager) -> Self {
        Self {
            ble: Mutex::new(ble),
            passthrough: Mutex::new(PassthroughFlags::default()),
            lock_active: AtomicBool::new(false),
            last_device_id: Mutex::new(None),
        }
    }

    pub fn is_locked(&self) -> bool {
        self.lock_active.load(Ordering::SeqCst)
    }

    pub fn set_locked(&self, active: bool) {
        self.lock_active.store(active, Ordering::SeqCst);
    }
}
