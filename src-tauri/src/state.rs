//! Shared application state: passthrough flags, lock state, the BLE manager,
//! and the input pipeline. Held behind `tauri::State` and shared across
//! commands.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::ble::BleManager;
use crate::input::{InputController, InputShared};

/// The three independent passthrough flags (plan §4.3). When a flag is off the
/// corresponding channel is *not* grabbed and behaves normally on the operator
/// OS; when on it is grabbed and forwarded to the host.
///
/// The `Default` (all `false`) is the conservative state: forward nothing until
/// the operator opts a channel in.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PassthroughFlags {
    pub keyboard: bool,
    pub mouse: bool,
    pub video: bool,
}

/// Root application state managed by Tauri.
///
/// `input_shared` holds lock-free atomics read by the grab callback's hot path
/// and is therefore kept outside any mutex; `input_ctl` serialises one-time
/// pipeline startup.
pub struct AppState {
    pub ble: Mutex<BleManager>,
    pub passthrough: Mutex<PassthroughFlags>,
    pub input_shared: Arc<InputShared>,
    pub input_ctl: Mutex<InputController>,
    /// Opaque, machine-local `Peripheral::id()` of the last device we connected
    /// to, for auto-reconnect. **Not** portable across machines (plan §4.1).
    pub last_device_id: Mutex<Option<String>>,
}

impl AppState {
    pub fn new(ble: BleManager) -> Self {
        let input_shared = Arc::new(InputShared::new());
        let input_ctl = Mutex::new(InputController::new(input_shared.clone()));
        Self {
            ble: Mutex::new(ble),
            passthrough: Mutex::new(PassthroughFlags::default()),
            input_shared,
            input_ctl,
            last_device_id: Mutex::new(None),
        }
    }

    pub fn is_locked(&self) -> bool {
        self.input_shared.is_locked()
    }

    pub fn set_locked(&self, active: bool) {
        self.input_shared.set_locked(active);
    }
}
