//! Backend → frontend event emitters (plan §7). Each event name is a constant
//! so the frontend's `listen` calls and the backend stay in lockstep.

use tauri::{AppHandle, Emitter};

use super::{ConnectionState, DiscoveredDevice, LedReport};

pub const DEVICES_CHANGED: &str = "devices_changed";
pub const CONNECTION_STATE: &str = "connection_state";
pub const LOCK_STATE: &str = "lock_state";
pub const KEYBOARD_LEDS: &str = "keyboard_leds";
pub const ERROR: &str = "error";

/// Structured error payload for the `error` event.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ErrorEvent {
    pub code: String,
    pub message: String,
}

pub fn devices_changed(app: &AppHandle, devices: &[DiscoveredDevice]) {
    let _ = app.emit(DEVICES_CHANGED, devices);
}

pub fn connection_state(app: &AppHandle, state: ConnectionState) {
    let _ = app.emit(CONNECTION_STATE, state);
}

pub fn lock_state(app: &AppHandle, active: bool) {
    let _ = app.emit(LOCK_STATE, active);
}

pub fn keyboard_leds(app: &AppHandle, leds: LedReport) {
    let _ = app.emit(KEYBOARD_LEDS, leds);
}

pub fn error(app: &AppHandle, code: &str, message: impl Into<String>) {
    let _ = app.emit(
        ERROR,
        ErrorEvent {
            code: code.to_string(),
            message: message.into(),
        },
    );
}
