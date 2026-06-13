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
    // M2/M3: re-evaluate which channels the grab thread should consume.
    Ok(())
}

#[tauri::command]
pub async fn enter_lock(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    // M2 installs the rdev grab here; for now we only own and broadcast state.
    state.set_locked(true);
    events::lock_state(&app, true);
    Ok(())
}

#[tauri::command]
pub async fn exit_lock(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.set_locked(false);
    // Guarantee the host never has stuck keys/buttons after lock exit (plan §9).
    state.ble.lock().await.release_all().await;
    events::lock_state(&app, false);
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
