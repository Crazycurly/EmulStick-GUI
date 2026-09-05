//! BLE data-plane: `btleplug` scanning, connection, Device Info readout, and
//! write-without-response to the F801/F803 characteristics (plan §4.1, M1).
//!
//! Identity note: a device is keyed by `Peripheral::id().to_string()`, an
//! opaque per-machine value (BD_ADDR on Windows, an OS-assigned UUID on
//! macOS). It is **not** a portable MAC address (plan §4.1).
//!
//! Locking shape (plan §9): the multi-second connect/reconnect orchestration
//! holds the [`BleManager`] mutex, but the *active connection handle* lives in a
//! separately-locked [`ConnHandle`] (a cheap std `RwLock`). The input writer and
//! the read/debug commands snapshot the handle without `.await`, so a slow
//! connect can never stall the hot write path or the other commands.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use btleplug::api::{
    Central, CentralEvent, Characteristic, Manager as _, Peripheral as _, PeripheralProperties,
    ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
// `as _` brings the `state()` extension method into scope without clashing with
// btleplug's `platform::Manager` imported above.
use tauri::{AppHandle, Manager as _};

use crate::ipc::events;
use crate::ipc::{ConnectionState, DeviceInfo, DiscoveredDevice, LedReport};
use crate::protocol::{uuids, KEYBOARD_RELEASE_ALL, MOUSE_RELEASE_ALL};
use crate::state::AppState;

/// Substring (case-insensitive) used as a secondary scan filter for devices
/// that don't advertise the F800 service UUID in their advertisement packet.
const NAME_HINT: &str = "emul";

/// Cap on a single GATT connect + service-discovery round (plan §9). Without it
/// CoreBluetooth can leave `connect()`/`discover_services()` pending forever,
/// silently defeating the frontend's reconnect backoff (it awaits a call that
/// never returns). On timeout we surface an error so the backoff retries.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Cap on a single data-plane GATT write (and the DIS reads). Write-without-
/// response normally returns once the report is queued to the OS, but a wedged
/// adapter/link can leave it pending forever — which would park the input writer
/// indefinitely. On timeout the write returns `Err`, so the writer's safe-state
/// path (`reset_input_state`) engages instead of hanging (plan §9). Comfortably
/// above any realistic BLE connection interval.
const WRITE_TIMEOUT: Duration = Duration::from_secs(1);

/// How long a user-initiated scan runs before auto-stopping. A scan keeps the
/// radio busy and the poller running; if the operator walks away we shouldn't
/// scan forever. The UI flips back to idle via the `Disconnected` event.
const SCAN_DURATION: Duration = Duration::from_secs(30);

/// How often the scan poller re-enumerates discovered peripherals.
const SCAN_POLL_INTERVAL: Duration = Duration::from_millis(1200);

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// A connected EmulStick: the peripheral plus its located write channels.
///
/// Held as `Arc<ConnectedDevice>` behind a [`ConnHandle`]. All writes are
/// time-bounded ([`WRITE_TIMEOUT`]) so a stuck link can't hang the caller.
pub struct ConnectedDevice {
    peripheral: Peripheral,
    keyboard: Characteristic,
    mouse: Characteristic,
    info: DeviceInfo,
}

impl ConnectedDevice {
    /// Write-without-response, bounded by [`WRITE_TIMEOUT`].
    async fn timed_write(&self, ch: &Characteristic, report: &[u8]) -> Result<(), String> {
        tokio::time::timeout(
            WRITE_TIMEOUT,
            self.peripheral.write(ch, report, WriteType::WithoutResponse),
        )
        .await
        .map_err(|_| "GATT write timed out".to_string())?
        .map_err(err)
    }

    /// Write an 8-byte keyboard report (F801).
    pub async fn write_keyboard(&self, report: &[u8]) -> Result<(), String> {
        self.timed_write(&self.keyboard, report).await
    }

    /// Write a 6-byte mouse report (F803).
    pub async fn write_mouse(&self, report: &[u8]) -> Result<(), String> {
        self.timed_write(&self.mouse, report).await
    }

    /// Best-effort safe state: release all keys and buttons (plan §9). Errors
    /// are swallowed because this runs on teardown paths.
    pub async fn write_release_all(&self) {
        let _ = self.timed_write(&self.keyboard, &KEYBOARD_RELEASE_ALL).await;
        let _ = self.timed_write(&self.mouse, &MOUSE_RELEASE_ALL).await;
    }
}

/// Cheap, shared handle to the active connection. Cloned into both [`AppState`]
/// and the [`BleManager`]; the input writer and the read/debug commands snapshot
/// the current [`ConnectedDevice`] via a brief std `RwLock` read (no `.await`),
/// so they never contend with the `BleManager` mutex during a connect/reconnect.
///
/// The critical sections only clone an `Arc` or take an `Option`, so they cannot
/// panic and the lock can never be poisoned — hence the `expect`s are infallible.
#[derive(Clone, Default)]
pub struct ConnHandle(Arc<std::sync::RwLock<Option<Arc<ConnectedDevice>>>>);

impl ConnHandle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot the current connection (clones an `Arc`; no `.await`).
    fn current(&self) -> Option<Arc<ConnectedDevice>> {
        self.0.read().expect("ConnHandle poisoned").clone()
    }

    /// The committed connection's device id, if any (no `.await`).
    fn current_id(&self) -> Option<String> {
        self.0
            .read()
            .expect("ConnHandle poisoned")
            .as_ref()
            .map(|d| d.peripheral.id().to_string())
    }

    fn set(&self, dev: ConnectedDevice) {
        *self.0.write().expect("ConnHandle poisoned") = Some(Arc::new(dev));
    }

    /// Clear and return the active connection (so the caller can disconnect it).
    fn take(&self) -> Option<Arc<ConnectedDevice>> {
        self.0.write().expect("ConnHandle poisoned").take()
    }

    pub fn is_connected(&self) -> bool {
        self.0.read().expect("ConnHandle poisoned").is_some()
    }

    pub fn device_info(&self) -> Option<DeviceInfo> {
        self.current().map(|d| d.info.clone())
    }

    /// Write an 8-byte keyboard report to the active connection.
    pub async fn write_keyboard(&self, report: &[u8]) -> Result<(), String> {
        match self.current() {
            Some(dev) => dev.write_keyboard(report).await,
            None => Err("Not connected".to_string()),
        }
    }

    /// Write a 6-byte mouse report to the active connection.
    pub async fn write_mouse(&self, report: &[u8]) -> Result<(), String> {
        match self.current() {
            Some(dev) => dev.write_mouse(report).await,
            None => Err("Not connected".to_string()),
        }
    }

    /// Best-effort release-all on the active connection (no-op if disconnected).
    pub async fn release_all(&self) {
        if let Some(dev) = self.current() {
            dev.write_release_all().await;
        }
    }
}

/// Owns the BLE adapter and orchestrates scan/connect/disconnect. The active
/// connection itself lives in the shared [`ConnHandle`] (`conn`), not here, so
/// the slow orchestration this struct's mutex guards never blocks data writes.
pub struct BleManager {
    adapter: Option<Adapter>,
    manager: Option<Manager>,
    conn: ConnHandle,
    /// Bumped on every `begin_connect`/`disconnect` so a slow connect attempt
    /// whose result arrives after a newer connect (or an intervening disconnect)
    /// can detect it was superseded and discard itself instead of clobbering the
    /// current connection.
    connect_gen: u64,
    /// Device id the in-flight connect is targeting (set by `begin_connect`,
    /// cleared by `disconnect`). Lets a superseded attempt tell "another attempt
    /// wants this same device" from "this device is now unwanted", so it only
    /// tears down a genuinely stray link (btleplug disconnect is keyed by device
    /// UUID, so a same-UUID disconnect would kill a concurrent same-device link).
    connecting_id: Option<String>,
    scan_stop: Option<Arc<AtomicBool>>,
    scan_task: Option<tauri::async_runtime::JoinHandle<()>>,
    /// Watches the active connection and emits `Disconnected` on an unexpected
    /// drop (aborted on intentional disconnect so it stays quiet).
    monitor_task: Option<tauri::async_runtime::JoinHandle<()>>,
}

impl BleManager {
    pub fn new(conn: ConnHandle) -> Self {
        Self {
            adapter: None,
            manager: None,
            conn,
            connect_gen: 0,
            connecting_id: None,
            scan_stop: None,
            scan_task: None,
            monitor_task: None,
        }
    }

    pub fn is_connected(&self) -> bool {
        self.conn.is_connected()
    }

    /// Lazily acquire the first system Bluetooth adapter. Kept out of
    /// construction so app startup never blocks on the radio.
    async fn ensure_adapter(&mut self) -> Result<Adapter, String> {
        if let Some(adapter) = &self.adapter {
            return Ok(adapter.clone());
        }
        let manager = Manager::new().await.map_err(err)?;
        let adapter = manager
            .adapters()
            .await
            .map_err(err)?
            .into_iter()
            .next()
            .ok_or_else(|| "No Bluetooth adapter found".to_string())?;
        self.manager = Some(manager);
        self.adapter = Some(adapter.clone());
        Ok(adapter)
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
                let deadline = tokio::time::Instant::now() + SCAN_DURATION;
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
                    // Auto-stop so the radio doesn't scan forever if the
                    // operator never connects (plan §10). The UI returns to idle
                    // on the `Disconnected` event.
                    if tokio::time::Instant::now() >= deadline {
                        let _ = adapter.stop_scan().await;
                        events::connection_state(&app, ConnectionState::Disconnected);
                        break;
                    }
                    tokio::time::sleep(SCAN_POLL_INTERVAL).await;
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

    /// Phase 1 of a connect (plan §9): acquire the adapter, stop any user scan,
    /// claim a fresh generation token, and announce `Connecting`. Returns the
    /// adapter (cloned) + token so the caller can run the slow connect work
    /// *without* holding this manager's mutex.
    pub async fn begin_connect(
        &mut self,
        app: &AppHandle,
        id: &str,
    ) -> Result<(Adapter, u64), String> {
        let adapter = self.ensure_adapter().await?;
        self.scan_stop().await?;
        self.connect_gen = self.connect_gen.wrapping_add(1);
        self.connecting_id = Some(id.to_string());
        events::connection_state(app, ConnectionState::Connecting);
        Ok((adapter, self.connect_gen))
    }

    /// Phase 3 of a connect: commit the established connection iff this attempt
    /// wasn't superseded (by a newer connect or an intervening disconnect). On
    /// supersession the freshly-connected peripheral is dropped cleanly so we
    /// never leave a stray link or clobber the current connection.
    pub async fn finish_connect(
        &mut self,
        app: &AppHandle,
        adapter: &Adapter,
        dev: ConnectedDevice,
        token: u64,
    ) -> Result<DeviceInfo, String> {
        if self.connect_gen != token {
            // Superseded by a newer connect or an intervening disconnect. Only
            // tear down the link if no current/committed connection and no newer
            // in-flight attempt wants this same device — btleplug's disconnect is
            // keyed by device UUID, so disconnecting a same-UUID superseded handle
            // would kill the live connection a concurrent same-device attempt just
            // committed (or is about to). Stray cross-device links are still cleaned.
            let dev_id = dev.peripheral.id().to_string();
            let still_wanted = self.conn.current_id().as_deref() == Some(&dev_id)
                || self.connecting_id.as_deref() == Some(&dev_id);
            if !still_wanted {
                let _ = dev.peripheral.disconnect().await;
            }
            return Err("Connection superseded".to_string());
        }
        let info = dev.info.clone();
        let id = dev.peripheral.id().to_string();
        self.abort_monitor();
        self.monitor_task = Some(spawn_connection_monitor(app, adapter, &id));
        self.conn.set(dev);
        // This attempt resolved: the committed connection's `current_id` now
        // covers "is this device wanted", so drop the in-flight claim. (We're the
        // current generation here, so we can't be clobbering a newer attempt.)
        self.connecting_id = None;
        events::connection_state(app, ConnectionState::Connected);
        Ok(info)
    }

    /// A connect attempt failed in [`establish`] before reaching
    /// [`Self::finish_connect`]. Drop its in-flight claim *iff* it's still the
    /// current generation, so a later superseded same-device attempt doesn't
    /// mistake this abandoned one for "still wanted" and skip tearing down its
    /// orphaned link. Token-guarded so it can't clear a newer attempt's claim.
    pub fn connect_failed(&mut self, token: u64) {
        if self.connect_gen == token {
            self.connecting_id = None;
        }
    }

    fn abort_monitor(&mut self) {
        if let Some(task) = self.monitor_task.take() {
            task.abort();
        }
    }

    /// Disconnect, first sending the safe all-up reports so nothing is left
    /// stuck pressed on the host (plan §9). Bumps the generation so any
    /// in-flight connect attempt discards itself in `finish_connect`.
    pub async fn disconnect(&mut self, app: &AppHandle) -> Result<(), String> {
        self.connect_gen = self.connect_gen.wrapping_add(1);
        // Mark no device as wanted, so a superseded in-flight attempt for the
        // same device tears its link down instead of leaving it connected.
        self.connecting_id = None;
        // Stop the monitor first so the intentional drop doesn't fire it.
        self.abort_monitor();
        self.stop_scan_task();
        if let Some(dev) = self.conn.take() {
            dev.write_release_all().await;
            let _ = dev.peripheral.disconnect().await;
        }
        events::connection_state(app, ConnectionState::Disconnected);
        Ok(())
    }

    pub fn device_info(&self) -> Option<DeviceInfo> {
        self.conn.device_info()
    }
}

/// Phase 2 of a connect (plan §9): the slow part, run with **no** manager mutex
/// held. Locates the device (scanning briefly if it isn't cached, so this also
/// serves reconnect), connects, discovers services, sends a clean-state
/// release-all, reads Device Info, and subscribes to LED notifications.
pub async fn establish(
    app: &AppHandle,
    adapter: &Adapter,
    id: &str,
) -> Result<ConnectedDevice, String> {
    let peripheral = find_or_scan(adapter, id).await?;

    if !peripheral.is_connected().await.map_err(err)? {
        tokio::time::timeout(CONNECT_TIMEOUT, peripheral.connect())
            .await
            .map_err(|_| "Connection attempt timed out".to_string())?
            .map_err(err)?;
    }
    tokio::time::timeout(CONNECT_TIMEOUT, peripheral.discover_services())
        .await
        .map_err(|_| "Service discovery timed out".to_string())?
        .map_err(err)?;

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

    // Start every (re)connection from a clean host state: clear any key or
    // button left stuck if a previous link dropped mid-press (plan §9
    // safe-state-on-failure — the release we couldn't send then lands now).
    let _ = timed_write(&peripheral, &keyboard, &KEYBOARD_RELEASE_ALL).await;
    let _ = timed_write(&peripheral, &mouse, &MOUSE_RELEASE_ALL).await;

    let info = read_device_info(&peripheral).await;
    spawn_led_listener(app, &peripheral, &keyboard).await;

    Ok(ConnectedDevice {
        peripheral,
        keyboard,
        mouse,
        info,
    })
}

/// [`WRITE_TIMEOUT`]-bounded write helper for the connect path (before a
/// `ConnectedDevice` exists). Mirrors [`ConnectedDevice::timed_write`].
async fn timed_write(p: &Peripheral, ch: &Characteristic, report: &[u8]) -> Result<(), String> {
    tokio::time::timeout(WRITE_TIMEOUT, p.write(ch, report, WriteType::WithoutResponse))
        .await
        .map_err(|_| "GATT write timed out".to_string())?
        .map_err(err)
}

/// Find a peripheral by its opaque id string among those the adapter knows.
async fn find_peripheral(adapter: &Adapter, id: &str) -> Option<Peripheral> {
    adapter
        .peripherals()
        .await
        .ok()?
        .into_iter()
        .find(|p| p.id().to_string() == id)
}

/// Locate a device by id, scanning briefly if it isn't already cached (so this
/// works both for a fresh connect and for reconnecting after a drop).
async fn find_or_scan(adapter: &Adapter, id: &str) -> Result<Peripheral, String> {
    if let Some(p) = find_peripheral(adapter, id).await {
        return Ok(p);
    }
    let _ = adapter.start_scan(ScanFilter::default()).await;
    let mut found = None;
    for _ in 0..12 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Some(p) = find_peripheral(adapter, id).await {
            found = Some(p);
            break;
        }
    }
    let _ = adapter.stop_scan().await;
    found.ok_or_else(|| "Device not found — is it powered on and in range?".to_string())
}

/// Watch the adapter's event stream and emit `Disconnected` when our device
/// drops, then exit. CoreBluetooth's `DeviceDisconnected` is the reliable
/// signal — polling `is_connected()` does not detect link loss on macOS.
/// Aborted by [`BleManager::abort_monitor`] on an intentional disconnect.
fn spawn_connection_monitor(
    app: &AppHandle,
    adapter: &Adapter,
    id: &str,
) -> tauri::async_runtime::JoinHandle<()> {
    let app = app.clone();
    let adapter = adapter.clone();
    let target = id.to_string();
    tauri::async_runtime::spawn(async move {
        let mut stream = match adapter.events().await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(?e, "adapter.events() failed; no disconnect detection");
                return;
            }
        };
        tracing::debug!("connection monitor (event stream) started");
        while let Some(event) = stream.next().await {
            tracing::trace!(?event, "central event");
            if let CentralEvent::DeviceDisconnected(pid) = event {
                if pid.to_string() == target {
                    tracing::info!("device disconnected → exiting lock + reconnecting");
                    // Free the operator FIRST and synchronously: never leave a
                    // frozen cursor / grabbed keyboard because the dongle
                    // vanished mid-lock (plan §4.2/§8). Exiting lock also makes
                    // the grab transparent, so it stops feeding the writer and
                    // the frontend-driven reconnect can make progress.
                    let state = app.state::<AppState>();
                    crate::input::emergency_unlock(&state.input_shared);
                    events::lock_state(&app, false);
                    events::connection_state(&app, ConnectionState::Disconnected);
                    break;
                }
            }
        }
    })
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
/// missing or unreadable (each read is independently time-bounded).
async fn read_device_info(p: &Peripheral) -> DeviceInfo {
    let chars = p.characteristics();
    let mut info = DeviceInfo::default();

    if let Some(c) = chars.iter().find(|c| c.uuid == uuids::FIRMWARE_REVISION) {
        if let Some(bytes) = timed_read(p, c).await {
            info.firmware = Some(clean_string(&bytes));
        }
    }
    if let Some(c) = chars.iter().find(|c| c.uuid == uuids::MODEL_NUMBER) {
        if let Some(bytes) = timed_read(p, c).await {
            info.model = Some(clean_string(&bytes));
        }
    }
    if let Some(c) = chars.iter().find(|c| c.uuid == uuids::MANUFACTURER_NAME) {
        if let Some(bytes) = timed_read(p, c).await {
            info.manufacturer = Some(clean_string(&bytes));
        }
    }
    if let Some(c) = chars.iter().find(|c| c.uuid == uuids::SYSTEM_ID) {
        if let Some(bytes) = timed_read(p, c).await {
            info.system_id = Some(hex_string(&bytes));
        }
    }
    info
}

/// [`WRITE_TIMEOUT`]-bounded DIS read; logs (rather than swallows silently) the
/// failure so an all-`None` `DeviceInfo` is diagnosable.
async fn timed_read(p: &Peripheral, c: &Characteristic) -> Option<Vec<u8>> {
    match tokio::time::timeout(WRITE_TIMEOUT, p.read(c)).await {
        Ok(Ok(bytes)) => Some(bytes),
        Ok(Err(e)) => {
            tracing::debug!(uuid = %c.uuid, ?e, "device-info read failed");
            None
        }
        Err(_) => {
            tracing::debug!(uuid = %c.uuid, "device-info read timed out");
            None
        }
    }
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
