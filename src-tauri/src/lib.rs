//! EmulStick desktop backend (Tauri 2). Control plane lives in the Svelte
//! frontend; this crate is the data plane — BLE transport, OS input hooks
//! (M2), and the HID report protocol.

pub mod ble;
pub mod input;
pub mod ipc;
pub mod protocol;
pub mod state;

use std::time::Duration;

use tauri::{Manager, RunEvent};

use state::AppState;

/// Bound on the exit teardown so quitting never hangs on a wedged radio. Sized
/// with headroom over the two final release-all writes (each ≤ `WRITE_TIMEOUT`).
const EXIT_TEARDOWN_TIMEOUT: Duration = Duration::from_secs(3);

/// Build and run the Tauri application.
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("emulstick=info,btleplug=warn")),
        )
        .init();

    // A panic anywhere — including on the grab thread — must never leave the
    // operator's cursor frozen/hidden (plan §8). Restore it before unwinding.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        input::release_cursor_capture();
        default_hook(info);
    }));

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            ipc::commands::scan_start,
            ipc::commands::scan_stop,
            ipc::commands::connect,
            ipc::commands::disconnect,
            ipc::commands::get_device_info,
            ipc::commands::get_passthrough,
            ipc::commands::set_passthrough,
            ipc::commands::set_target_os,
            ipc::commands::enter_lock,
            ipc::commands::exit_lock,
            ipc::commands::check_accessibility,
            ipc::commands::open_accessibility_settings,
            ipc::commands::debug_send_keyboard,
            ipc::commands::debug_send_mouse,
            ipc::commands::debug_tap_key,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        // On exit, guarantee the operator's machine is never left captured
        // (plan §8/§9): unfreeze the cursor and release all keys/buttons so
        // nothing is stuck pressed on the host. Bounded by a timeout so a
        // wedged BLE link can't hang the quit.
        if let RunEvent::ExitRequested { .. } = event {
            input::release_cursor_capture();
            let state = app_handle.state::<AppState>();
            tauri::async_runtime::block_on(async {
                let _ = tokio::time::timeout(
                    EXIT_TEARDOWN_TIMEOUT,
                    input::shutdown(&state),
                )
                .await;
            });
        }
    });
}
