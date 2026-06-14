//! Tauri command handlers (frontend → backend). Kept low-frequency per plan
//! §3; high-rate input never crosses this boundary.

use tauri::{AppHandle, State};

use crate::ipc::{events, ConnectionState, DeviceInfo};
use crate::protocol::{keyboard::KEYBOARD_REPORT_LEN, mouse::MOUSE_REPORT_LEN};
use crate::state::{AppState, PassthroughFlags};

#[tauri::command]
pub async fn scan_start(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.ble.lock().await.scan_start(&app).await
}

#[tauri::command]
pub async fn scan_stop(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.ble.lock().await.scan_stop().await?;
    // The manager's `scan_stop` is event-free so internal callers (connect)
    // don't flicker; a user-initiated stop returns the UI to idle here.
    events::connection_state(&app, ConnectionState::Disconnected);
    Ok(())
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
    let prev = {
        let mut guard = state.passthrough.lock().await;
        let prev = *guard;
        *guard = flags;
        prev
    };
    // Mirror to the grab callback so it gates channels live (plan §4.3).
    state.input_shared.set_passthrough(flags.keyboard, flags.mouse);
    // Refresh relative-mouse capture, and if a channel was switched off while
    // locked, release whatever it left held on the host (plan §9).
    crate::input::on_passthrough_changed(&state.input_shared, prev, flags);
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

/// Whether the OS trusts this process to capture global input (macOS
/// Accessibility, plan §5). Always true on non-macOS. When `prompt` is true and
/// the process isn't yet trusted, macOS shows its "allow to control" dialog and
/// adds the app to the Accessibility list — the onboarding entry point.
#[tauri::command]
pub fn check_accessibility(prompt: bool) -> bool {
    crate::input::accessibility_trusted(prompt)
}

/// Open the macOS Accessibility settings pane so the operator can grant the
/// permission. No-op on other platforms.
#[tauri::command]
pub fn open_accessibility_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn()
            .map_err(|e| e.to_string())?;
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
