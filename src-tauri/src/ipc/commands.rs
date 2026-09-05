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
    // Phase 1 (brief lock): adapter + stop scan + claim a generation token.
    let (adapter, token) = {
        let mut mgr = state.ble.lock().await;
        mgr.begin_connect(&app, &device_id).await?
    };
    // Phase 2 (NO manager lock): the slow connect/discover/read. The input
    // writer and every other command stay responsive throughout.
    let dev = match crate::ble::establish(&app, &adapter, &device_id).await {
        Ok(dev) => dev,
        Err(e) => {
            // Drop this attempt's in-flight claim (iff still current) so a later
            // superseded same-device finish_connect doesn't skip cleaning up an
            // orphaned link, then surface the failure as `Disconnected` so the UI
            // doesn't stick on "Connecting…" (the reconnect backoff drives retry).
            state.ble.lock().await.connect_failed(token);
            events::connection_state(&app, ConnectionState::Disconnected);
            return Err(e);
        }
    };
    // Phase 3 (brief lock): commit iff this attempt wasn't superseded by a newer
    // connect or an intervening disconnect.
    let mut mgr = state.ble.lock().await;
    mgr.finish_connect(&app, &adapter, dev, token).await
}

#[tauri::command]
pub async fn disconnect(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.ble.lock().await.disconnect(&app).await
}

#[tauri::command]
pub async fn get_device_info(state: State<'_, AppState>) -> Result<Option<DeviceInfo>, String> {
    // Reads the connection handle directly (no BLE-manager lock), so it never
    // blocks behind an in-flight connect.
    Ok(state.conn.device_info())
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
    // Hold the guard across the mirror + reaction so a concurrent call (or a
    // racing enter_lock) can't act on a stale `prev`. Both calls below are
    // synchronous (no `.await`), so the guard is never held across a yield.
    let mut guard = state.passthrough.lock().await;
    let prev = *guard;
    *guard = flags;
    // Mirror to the grab callback so it gates channels live (plan §4.3).
    state.input_shared.set_passthrough(flags.keyboard, flags.mouse);
    // Refresh relative-mouse capture, and if a channel was switched off while
    // locked, release whatever it left held on the host (plan §9).
    crate::input::on_passthrough_changed(&state.input_shared, prev, flags);
    Ok(())
}

/// Select the **target system**'s OS for modifier remapping. When it differs from
/// the host this app runs on, forwarded keys swap Alt↔Win (Alt→⌘, Win→⌥; Ctrl
/// stays Control) so the operator's modifiers line up across the keyboard
/// mismatch; a matching OS forwards 1:1.
#[tauri::command]
pub async fn set_target_os(state: State<'_, AppState>, mac: bool) -> Result<(), String> {
    // Changing this while locked releases any held key first (see the function),
    // so a modifier can't span two mappings and stick.
    state.input_shared.set_target_mac(mac);
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
        // Pipeline never started, so nothing was ever grabbed or forwarded —
        // just confirm the (already-unlocked) state to the UI (plan §9).
        state.set_locked(false);
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
    state.conn.write_keyboard(&report).await
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
    state.conn.write_mouse(&report).await
}

/// Debug: press then release a single HID usage id, so the UI can "type" a key
/// without constructing reports (M1).
#[tauri::command]
pub async fn debug_tap_key(state: State<'_, AppState>, usage: u8) -> Result<(), String> {
    use crate::protocol::KeyboardState;
    let mut kb = KeyboardState::new();
    kb.press_key(usage);
    state.conn.write_keyboard(&kb.report()).await?;
    kb.release_key(usage);
    state.conn.write_keyboard(&kb.report()).await
}
