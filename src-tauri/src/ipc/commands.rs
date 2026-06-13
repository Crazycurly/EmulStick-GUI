//! Tauri command handlers (frontend → backend). Kept low-frequency per plan
//! §3; high-rate input never crosses this boundary.

use tauri::{AppHandle, State};

use crate::ipc::{events, DeviceInfo};
use crate::protocol::{keyboard::KEYBOARD_REPORT_LEN, mouse::MOUSE_REPORT_LEN};
use crate::state::{AppState, PassthroughFlags};

#[tauri::command]
pub async fn scan_start(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.ble.lock().await.scan_start(&app).await
}

#[tauri::command]
pub async fn scan_stop(state: State<'_, AppState>) -> Result<(), String> {
    state.ble.lock().await.scan_stop().await
}

#[tauri::command]
pub async fn connect(
    app: AppHandle,
    state: State<'_, AppState>,
    device_id: String,
) -> Result<DeviceInfo, String> {
    let info = state.ble.lock().await.connect(&app, &device_id).await?;
    *state.last_device_id.lock().await = Some(device_id);
    Ok(info)
}

#[tauri::command]
pub async fn disconnect(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.ble.lock().await.disconnect(&app).await
}

#[tauri::command]
pub async fn get_device_info(state: State<'_, AppState>) -> Result<Option<DeviceInfo>, String> {
    Ok(state.ble.lock().await.device_info())
}

#[tauri::command]
pub async fn get_passthrough(state: State<'_, AppState>) -> Result<PassthroughFlags, String> {
    Ok(*state.passthrough.lock().await)
}

#[tauri::command]
pub async fn set_passthrough(
    state: State<'_, AppState>,
    flags: PassthroughFlags,
) -> Result<(), String> {
    *state.passthrough.lock().await = flags;
    // Mirror to the grab callback so it gates channels live (plan §4.3).
    state.input_shared.set_passthrough(flags.keyboard, flags.mouse);
    Ok(())
}

#[tauri::command]
pub async fn enter_lock(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    // Install the grab hook on first use, then route the transition through the
    // writer task (it owns the canonical state change + event emission).
    state.input_ctl.lock().await.ensure_started(&app);
    // Make sure the callback sees the current passthrough flags immediately.
    let flags = *state.passthrough.lock().await;
    state.input_shared.set_passthrough(flags.keyboard, flags.mouse);
    state.input_shared.request_lock(true);
    Ok(())
}

#[tauri::command]
pub async fn exit_lock(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    if state.input_shared.pipeline_started() {
        // The writer emits lock_state and sends the safe all-up reports.
        state.input_shared.request_lock(false);
    } else {
        // Pipeline never started — fall back to a direct safe state (plan §9).
        state.set_locked(false);
        state.ble.lock().await.release_all().await;
        events::lock_state(&app, false);
    }
    Ok(())
}

/// Debug: write a raw 8-byte keyboard report (M1 — validates §6 encoders
/// against real hardware from the UI).
#[tauri::command]
pub async fn debug_send_keyboard(
    state: State<'_, AppState>,
    report: Vec<u8>,
) -> Result<(), String> {
    if report.len() != KEYBOARD_REPORT_LEN {
        return Err(format!(
            "keyboard report must be {KEYBOARD_REPORT_LEN} bytes, got {}",
            report.len()
        ));
    }
    state.ble.lock().await.write_keyboard(&report).await
}

/// Debug: write a raw 6-byte mouse report (M1).
#[tauri::command]
pub async fn debug_send_mouse(state: State<'_, AppState>, report: Vec<u8>) -> Result<(), String> {
    if report.len() != MOUSE_REPORT_LEN {
        return Err(format!(
            "mouse report must be {MOUSE_REPORT_LEN} bytes, got {}",
            report.len()
        ));
    }
    state.ble.lock().await.write_mouse(&report).await
}

/// Debug: press then release a single HID usage id, so the UI can "type" a key
/// without constructing reports (M1).
#[tauri::command]
pub async fn debug_tap_key(state: State<'_, AppState>, usage: u8) -> Result<(), String> {
    use crate::protocol::KeyboardState;
    let ble = state.ble.lock().await;
    let mut kb = KeyboardState::new();
    kb.press_key(usage);
    ble.write_keyboard(&kb.report()).await?;
    kb.release_key(usage);
    ble.write_keyboard(&kb.report()).await
}
