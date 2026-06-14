// Typed wrappers around the Tauri command/event bridge (plan §7).
//
// Tauri 2 converts camelCase JS argument keys to snake_case Rust parameters,
// so `deviceId` here maps to `device_id` in the command signature.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  ConnectionState,
  DeviceInfo,
  DiscoveredDevice,
  ErrorEvent,
  LedReport,
  PassthroughFlags,
} from "./types";

export const commands = {
  scanStart: () => invoke<void>("scan_start"),
  scanStop: () => invoke<void>("scan_stop"),
  connect: (deviceId: string) => invoke<DeviceInfo>("connect", { deviceId }),
  disconnect: () => invoke<void>("disconnect"),
  getDeviceInfo: () => invoke<DeviceInfo | null>("get_device_info"),
  getPassthrough: () => invoke<PassthroughFlags>("get_passthrough"),
  setPassthrough: (flags: PassthroughFlags) =>
    invoke<void>("set_passthrough", { flags }),
  enterLock: () => invoke<void>("enter_lock"),
  exitLock: () => invoke<void>("exit_lock"),
  /** Whether the OS trusts us to capture global input (macOS Accessibility).
   *  Pass `prompt: true` to trigger macOS's grant dialog when not yet trusted. */
  checkAccessibility: (prompt: boolean) =>
    invoke<boolean>("check_accessibility", { prompt }),
  openAccessibilitySettings: () =>
    invoke<void>("open_accessibility_settings"),
  debugSendKeyboard: (report: number[]) =>
    invoke<void>("debug_send_keyboard", { report }),
  debugSendMouse: (report: number[]) =>
    invoke<void>("debug_send_mouse", { report }),
  debugTapKey: (usage: number) => invoke<void>("debug_tap_key", { usage }),
};

export const events = {
  onDevicesChanged: (cb: (d: DiscoveredDevice[]) => void): Promise<UnlistenFn> =>
    listen<DiscoveredDevice[]>("devices_changed", (e) => cb(e.payload)),
  onConnectionState: (cb: (s: ConnectionState) => void): Promise<UnlistenFn> =>
    listen<ConnectionState>("connection_state", (e) => cb(e.payload)),
  onLockState: (cb: (active: boolean) => void): Promise<UnlistenFn> =>
    listen<boolean>("lock_state", (e) => cb(e.payload)),
  onKeyboardLeds: (cb: (leds: LedReport) => void): Promise<UnlistenFn> =>
    listen<LedReport>("keyboard_leds", (e) => cb(e.payload)),
  onError: (cb: (err: ErrorEvent) => void): Promise<UnlistenFn> =>
    listen<ErrorEvent>("error", (e) => cb(e.payload)),
};
