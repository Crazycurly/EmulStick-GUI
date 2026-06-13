//! BLE data-plane: `btleplug` scanning, connection, Device Info readout, and
//! write-without-response to the F801/F803 characteristics (plan §4.1, M1).
//!
//! Identity note: a device is keyed by `Peripheral::id().to_string()`, an
//! opaque per-machine value (BD_ADDR on Windows, an OS-assigned UUID on
//! macOS). It is **not** a portable MAC address (plan §4.1).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use btleplug::api::{
    Central, Characteristic, Manager as _, Peripheral as _, PeripheralProperties, ScanFilter,
    WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
use tauri::AppHandle;

use crate::ipc::events;
use crate::ipc::{ConnectionState, DeviceInfo, DiscoveredDevice, LedReport};
use crate::protocol::{uuids, KEYBOARD_RELEASE_ALL, MOUSE_RELEASE_ALL};

/// Substring (case-insensitive) used as a secondary scan filter for devices
/// that don't advertise the F800 service UUID in their advertisement packet.
const NAME_HINT: &str = "emul";

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// A connected EmulStick: the peripheral plus its located write channels.
struct ConnectedDevice {
    peripheral: Peripheral,
    keyboard: Characteristic,
    mouse: Characteristic,
    info: DeviceInfo,
}

/// Owns the BLE adapter and (at most one) active connection. Held behind an
/// async `Mutex` in [`crate::state::AppState`].
pub struct BleManager {
    adapter: Option<Adapter>,
    manager: Option<Manager>,
    connected: Option<ConnectedDevice>,
    scan_stop: Option<Arc<AtomicBool>>,
    scan_task: Option<tauri::async_runtime::JoinHandle<()>>,
}

impl Default for BleManager {
    fn default() -> Self {
        Self::new()
    }
}

impl BleManager {
    pub fn new() -> Self {
        Self {
            adapter: None,
            manager: None,
            connected: None,
            scan_stop: None,
            scan_task: None,
        }
    }

    pub fn is_connected(&self) -> bool {
        self.connected.is_some()
    }

    /// Lazily acquire the first system Bluetooth adapter. Kept out of
    /// construction so app startup never blocks on the radio.
    async fn ensure_adapter(&mut self) -> Result<Adapter, String> {
        if self.adapter.is_none() {
            let manager = Manager::new().await.map_err(err)?;
            let adapter = manager
                .adapters()
                .await
                .map_err(err)?
                .into_iter()
                .next()
                .ok_or_else(|| "No Bluetooth adapter found".to_string())?;
            self.manager = Some(manager);
            self.adapter = Some(adapter);
        }
        Ok(self.adapter.clone().expect("adapter just set"))
    }

    /// Start scanning and spawn a poller that emits `devices_changed`.
    ///
    /// Scanning is intentionally unfiltered at the radio level (CoreBluetooth's
    /// service filter hides devices that don't advertise the UUID); filtering
    /// to EmulStick devices happens in [`is_emulstick`].
    pub async fn scan_start(&mut self, app: &AppHandle) -> Result<(), String> {
        let adapter = self.ensure_adapter().await?;
        adapter
            .start_scan(ScanFilter::default())
            .await
            .map_err(err)?;

        self.stop_scan_task();
        let stop = Arc::new(AtomicBool::new(false));
        let task = {
            let stop = stop.clone();
            let app = app.clone();
            let adapter = adapter.clone();
            tauri::async_runtime::spawn(async move {
                while !stop.load(Ordering::SeqCst) {
                    if let Ok(peripherals) = adapter.peripherals().await {
                        let mut devices = Vec::new();
                        for p in peripherals {
                            if let Ok(Some(props)) = p.properties().await {
                                if is_emulstick(&props) {
                                    devices.push(DiscoveredDevice {
                                        id: p.id().to_string(),
                                        name: props.local_name.clone(),
                                        rssi: props.rssi,
                                    });
                                }
                            }
                        }
                        events::devices_changed(&app, &devices);
                    }
                    tokio::time::sleep(Duration::from_millis(1200)).await;
                }
            })
        };
        self.scan_stop = Some(stop);
        self.scan_task = Some(task);
        events::connection_state(app, ConnectionState::Scanning);
        Ok(())
    }

    pub async fn scan_stop(&mut self) -> Result<(), String> {
        self.stop_scan_task();
        if let Some(adapter) = &self.adapter {
            let _ = adapter.stop_scan().await;
        }
        Ok(())
    }

    fn stop_scan_task(&mut self) {
        if let Some(stop) = self.scan_stop.take() {
            stop.store(true, Ordering::SeqCst);
        }
        if let Some(task) = self.scan_task.take() {
            task.abort();
        }
    }

    /// Connect to a previously-scanned device, locate the F801/F803 channels,
    /// read Device Info, and subscribe to LED notifications.
    pub async fn connect(&mut self, app: &AppHandle, id: &str) -> Result<DeviceInfo, String> {
        let adapter = self.ensure_adapter().await?;
        self.scan_stop().await?;
        events::connection_state(app, ConnectionState::Connecting);

        let peripheral = adapter
            .peripherals()
            .await
            .map_err(err)?
            .into_iter()
            .find(|p| p.id().to_string() == id)
            .ok_or_else(|| "Device not found — scan again".to_string())?;

        if !peripheral.is_connected().await.map_err(err)? {
            peripheral.connect().await.map_err(err)?;
        }
        peripheral.discover_services().await.map_err(err)?;

        let chars = peripheral.characteristics();
        let keyboard = chars
            .iter()
            .find(|c| c.uuid == uuids::KEYBOARD)
            .cloned()
            .ok_or_else(|| "Keyboard characteristic (F801) not found".to_string())?;
        let mouse = chars
            .iter()
            .find(|c| c.uuid == uuids::MOUSE)
            .cloned()
            .ok_or_else(|| "Mouse characteristic (F803) not found".to_string())?;

        let info = read_device_info(&peripheral).await;
        spawn_led_listener(app, &peripheral, &keyboard).await;

        self.connected = Some(ConnectedDevice {
            peripheral,
            keyboard,
            mouse,
            info: info.clone(),
        });
        events::connection_state(app, ConnectionState::Connected);
        Ok(info)
    }

    /// Disconnect, first sending the safe all-up reports so nothing is left
    /// stuck pressed on the host (plan §9).
    pub async fn disconnect(&mut self, app: &AppHandle) -> Result<(), String> {
        self.stop_scan_task();
        if let Some(dev) = self.connected.take() {
            let _ = dev
                .peripheral
                .write(&dev.keyboard, &KEYBOARD_RELEASE_ALL, WriteType::WithoutResponse)
                .await;
            let _ = dev
                .peripheral
                .write(&dev.mouse, &MOUSE_RELEASE_ALL, WriteType::WithoutResponse)
                .await;
            let _ = dev.peripheral.disconnect().await;
        }
        events::connection_state(app, ConnectionState::Disconnected);
        Ok(())
    }

    pub fn device_info(&self) -> Option<DeviceInfo> {
        self.connected.as_ref().map(|d| d.info.clone())
    }

    /// Write an 8-byte keyboard report (F801, write-without-response).
    pub async fn write_keyboard(&self, report: &[u8]) -> Result<(), String> {
        let dev = self.connected.as_ref().ok_or("Not connected")?;
        dev.peripheral
            .write(&dev.keyboard, report, WriteType::WithoutResponse)
            .await
            .map_err(err)
    }

    /// Write a 6-byte mouse report (F803, write-without-response).
    pub async fn write_mouse(&self, report: &[u8]) -> Result<(), String> {
        let dev = self.connected.as_ref().ok_or("Not connected")?;
        dev.peripheral
            .write(&dev.mouse, report, WriteType::WithoutResponse)
            .await
            .map_err(err)
    }

    /// Best-effort safe state: release all keys and buttons (plan §9). Errors
    /// are swallowed because this runs on teardown paths.
    pub async fn release_all(&self) {
        if let Some(dev) = self.connected.as_ref() {
            let _ = dev
                .peripheral
                .write(&dev.keyboard, &KEYBOARD_RELEASE_ALL, WriteType::WithoutResponse)
                .await;
            let _ = dev
                .peripheral
                .write(&dev.mouse, &MOUSE_RELEASE_ALL, WriteType::WithoutResponse)
                .await;
        }
    }
}

/// Keep a device if it advertises the Custom Service, or its name hints at an
/// EmulStick — covers both well-behaved and minimal advertisers.
fn is_emulstick(props: &PeripheralProperties) -> bool {
    if props.services.contains(&uuids::CUSTOM_SERVICE) {
        return true;
    }
    props
        .local_name
        .as_deref()
        .map(|n| n.to_lowercase().contains(NAME_HINT))
        .unwrap_or(false)
}

/// Read the Device Information Service characteristics, tolerating any that are
/// missing or unreadable.
async fn read_device_info(p: &Peripheral) -> DeviceInfo {
    let chars = p.characteristics();
    let mut info = DeviceInfo::default();

    if let Some(c) = chars.iter().find(|c| c.uuid == uuids::FIRMWARE_REVISION) {
        if let Ok(bytes) = p.read(c).await {
            info.firmware = Some(clean_string(&bytes));
        }
    }
    if let Some(c) = chars.iter().find(|c| c.uuid == uuids::MODEL_NUMBER) {
        if let Ok(bytes) = p.read(c).await {
            info.model = Some(clean_string(&bytes));
        }
    }
    if let Some(c) = chars.iter().find(|c| c.uuid == uuids::MANUFACTURER_NAME) {
        if let Ok(bytes) = p.read(c).await {
            info.manufacturer = Some(clean_string(&bytes));
        }
    }
    if let Some(c) = chars.iter().find(|c| c.uuid == uuids::SYSTEM_ID) {
        if let Ok(bytes) = p.read(c).await {
            info.system_id = Some(hex_string(&bytes));
        }
    }
    info
}

/// Subscribe to F801 LED notifications and forward them as `keyboard_leds`
/// events (NumLock/CapsLock/ScrollLock reflection, plan §4.1).
async fn spawn_led_listener(app: &AppHandle, peripheral: &Peripheral, keyboard: &Characteristic) {
    if peripheral.subscribe(keyboard).await.is_err() {
        return;
    }
    let Ok(mut stream) = peripheral.notifications().await else {
        return;
    };
    let app = app.clone();
    let kbd_uuid = keyboard.uuid;
    tauri::async_runtime::spawn(async move {
        while let Some(notification) = stream.next().await {
            if notification.uuid == kbd_uuid {
                if let Some(&byte) = notification.value.first() {
                    events::keyboard_leds(&app, LedReport::from_byte(byte));
                }
            }
        }
    });
}

fn clean_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_matches('\0')
        .trim()
        .to_string()
}

fn hex_string(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_string_formats_system_id() {
        assert_eq!(hex_string(&[0x01, 0xAB, 0xCD]), "01:AB:CD");
    }

    #[test]
    fn clean_string_strips_nul_and_whitespace() {
        assert_eq!(clean_string(b"1.3.0\0\0"), "1.3.0");
        assert_eq!(clean_string(b"  EmulStick "), "EmulStick");
    }
}
