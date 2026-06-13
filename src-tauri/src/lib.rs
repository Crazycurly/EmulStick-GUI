//! EmulStick desktop backend (Tauri 2). Control plane lives in the Svelte
//! frontend; this crate is the data plane — BLE transport, OS input hooks
//! (M2), and the HID report protocol.

pub mod ble;
pub mod input;
pub mod ipc;
pub mod protocol;
pub mod state;

use ble::BleManager;
use state::AppState;

/// Build and run the Tauri application.
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("emulstick=info,btleplug=warn")),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .manage(AppState::new(BleManager::new()))
        .invoke_handler(tauri::generate_handler![
            ipc::commands::scan_start,
            ipc::commands::scan_stop,
            ipc::commands::connect,
            ipc::commands::disconnect,
            ipc::commands::get_device_info,
            ipc::commands::get_passthrough,
            ipc::commands::set_passthrough,
            ipc::commands::enter_lock,
            ipc::commands::exit_lock,
            ipc::commands::debug_send_keyboard,
            ipc::commands::debug_send_mouse,
            ipc::commands::debug_tap_key,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
