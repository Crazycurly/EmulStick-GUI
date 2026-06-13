<script lang="ts">
  import { onMount } from "svelte";
  import type { UnlistenFn } from "@tauri-apps/api/event";
  import { load, type Store } from "@tauri-apps/plugin-store";
  import { commands, events } from "./lib/ipc";
  import type {
    ConnectionState,
    DeviceInfo,
    DiscoveredDevice,
    LedReport,
    PassthroughFlags,
  } from "./lib/types";

  type SavedDevice = { id: string; name: string | null };

  let connection = $state<ConnectionState>("Disconnected");
  let devices = $state<DiscoveredDevice[]>([]);
  let info = $state<DeviceInfo | null>(null);
  let passthrough = $state<PassthroughFlags>({
    keyboard: false,
    mouse: false,
    video: false,
  });
  let leds = $state<LedReport>({ num: false, caps: false, scroll: false });
  let locked = $state(false);
  let scanning = $state(false);
  let lastError = $state<string | null>(null);
  let reconnecting = $state(false);
  let savedDevice = $state<SavedDevice | null>(null);

  const connected = $derived(connection === "Connected");

  let store: Store | null = null;
  // Distinguishes a user-initiated disconnect (forget) from an unexpected drop.
  let intentional = false;
  // Bumped to cancel an in-flight reconnect loop.
  let reconnectToken = 0;

  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

  onMount(() => {
    const unlisteners: Promise<UnlistenFn>[] = [
      events.onDevicesChanged((d) => (devices = d)),
      events.onConnectionState((s) => {
        connection = s;
        if (s === "Connected") reconnecting = false;
        if (s === "Disconnected") {
          info = null;
          // Unexpected drop while we remember a device → auto-reconnect.
          if (!intentional && savedDevice) startReconnect();
        }
        scanning = s === "Scanning";
      }),
      events.onLockState((active) => (locked = active)),
      events.onKeyboardLeds((l) => (leds = l)),
      events.onError((e) => (lastError = `${e.code}: ${e.message}`)),
    ];

    commands.getPassthrough().then((p) => (passthrough = p));

    (async () => {
      store = await load("emulstick.json");
      savedDevice = (await store.get<SavedDevice>("lastDevice")) ?? null;
      // Already connected (e.g. after a frontend hot-reload)? Reflect it.
      const di = await commands.getDeviceInfo();
      if (di) {
        info = di;
        connection = "Connected";
      } else if (savedDevice) {
        // Otherwise auto-reconnect to the last device on startup.
        startReconnect();
      }
    })();

    return () => {
      for (const u of unlisteners) u.then((fn) => fn());
    };
  });

  async function run(action: () => Promise<unknown>) {
    lastError = null;
    try {
      await action();
    } catch (e) {
      lastError = String(e);
    }
  }

  const toggleScan = () =>
    run(scanning ? commands.scanStop : commands.scanStart);

  async function attemptConnect(id: string, name: string | null) {
    info = await commands.connect(id);
    savedDevice = { id, name };
    await store?.set("lastDevice", savedDevice);
    await store?.save();
    reconnecting = false;
  }

  const connect = (device: DiscoveredDevice) =>
    run(async () => {
      intentional = false;
      await attemptConnect(device.id, device.name);
    });

  /// Retry connecting to the saved device with bounded exponential backoff
  /// until it succeeds or the user cancels (plan §9).
  async function startReconnect() {
    if (reconnecting) return;
    reconnecting = true;
    const token = ++reconnectToken;
    let delay = 1000;
    while (reconnectToken === token && savedDevice) {
      try {
        await attemptConnect(savedDevice.id, savedDevice.name);
        return;
      } catch {
        if (reconnectToken !== token) return;
        await sleep(delay);
        delay = Math.min(delay * 2, 30000);
      }
    }
    reconnecting = false;
  }

  async function disconnect() {
    intentional = true;
    reconnectToken++; // cancel any reconnect loop
    reconnecting = false;
    savedDevice = null;
    await store?.delete("lastDevice");
    await store?.save();
    await run(commands.disconnect);
  }

  const toggleLock = () =>
    run(locked ? commands.exitLock : commands.enterLock);

  function updatePassthrough(key: keyof PassthroughFlags, value: boolean) {
    passthrough = { ...passthrough, [key]: value };
    run(() => commands.setPassthrough(passthrough));
  }

  // ── Debug helpers (M1: validate §6 encoders against real hardware) ──────
  const tapA = () => run(() => commands.debugTapKey(4)); // usage 4 = "a"

  // Win+Shift+S then release-all, mirroring docs/protocol.md Example 1.1.
  const winShiftS = () =>
    run(async () => {
      await commands.debugSendKeyboard([0xa0, 0, 0x16, 0, 0, 0, 0, 0]);
      await commands.debugSendKeyboard([0, 0, 0, 0, 0, 0, 0, 0]);
    });

  // Nudge the cursor down-right by (40, 40), then stop.
  const nudgeMouse = () =>
    run(async () => {
      await commands.debugSendMouse([0, 40, 0, 40, 0, 0]);
      await commands.debugSendMouse([0, 0, 0, 0, 0, 0]);
    });
</script>

<main>
  <header>
    <h1>EmulStick</h1>
    <span class="badge" class:connected class:locked class:reconnecting>
      {locked ? "LOCKED · " : ""}{reconnecting ? "Reconnecting…" : connection}
    </span>
  </header>

  {#if lastError}
    <p class="error" role="alert">{lastError}</p>
  {/if}

  {#if reconnecting}
    <div class="card reconnect-banner">
      <span>Reconnecting to <strong>{savedDevice?.name ?? "device"}</strong>…</span>
      <button class="secondary" onclick={disconnect}>Stop &amp; forget</button>
    </div>
  {/if}

  <section class="card">
    <div class="row">
      <h2>Devices</h2>
      <button onclick={toggleScan}>
        {scanning ? "Stop scan" : "Scan"}
      </button>
    </div>
    {#if devices.length === 0}
      <p class="muted">
        {scanning ? "Scanning…" : "No devices. Start a scan."}
      </p>
    {:else}
      <ul class="devices">
        {#each devices as device (device.id)}
          <li>
            <div>
              <strong>{device.name ?? "Unknown"}</strong>
              <span class="muted mono">{device.id}</span>
            </div>
            <div class="row">
              {#if device.rssi != null}
                <span class="muted">{device.rssi} dBm</span>
              {/if}
              <button
                onclick={() => connect(device)}
                disabled={connected || reconnecting}
              >
                Connect
              </button>
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  {#if connected}
    <section class="card">
      <div class="row">
        <h2>Connected device</h2>
        <button class="secondary" onclick={disconnect}>Disconnect</button>
      </div>
      <dl class="info">
        <dt>Firmware</dt>
        <dd>{info?.firmware ?? "—"}</dd>
        <dt>Model</dt>
        <dd>{info?.model ?? "—"}</dd>
        <dt>Manufacturer</dt>
        <dd>{info?.manufacturer ?? "—"}</dd>
        <dt>System ID</dt>
        <dd class="mono">{info?.systemId ?? "—"}</dd>
      </dl>
      <div class="leds">
        <span class="led" class:on={leds.num}>Num</span>
        <span class="led" class:on={leds.caps}>Caps</span>
        <span class="led" class:on={leds.scroll}>Scroll</span>
      </div>
    </section>

    <section class="card">
      <h2>Passthrough</h2>
      <label>
        <input
          type="checkbox"
          checked={passthrough.keyboard}
          onchange={(e) => updatePassthrough("keyboard", e.currentTarget.checked)}
        />
        Keyboard
      </label>
      <label>
        <input
          type="checkbox"
          checked={passthrough.mouse}
          onchange={(e) => updatePassthrough("mouse", e.currentTarget.checked)}
        />
        Mouse
      </label>
      <label>
        <input
          type="checkbox"
          checked={passthrough.video}
          onchange={(e) => updatePassthrough("video", e.currentTarget.checked)}
        />
        Video
      </label>
    </section>

    <section class="card">
      <div class="row">
        <h2>Lock mode</h2>
        <button class:danger={locked} onclick={toggleLock}>
          {locked ? "Exit lock" : "Enter lock"}
        </button>
      </div>
      <p class="muted">
        Lock-mode input grabbing arrives in M2. The hotkey will always release
        the lock so the operator can't get trapped.
      </p>
    </section>

    <section class="card">
      <h2>Debug · send reports</h2>
      <p class="muted">Validates the §6 encoders against the dongle.</p>
      <div class="row wrap">
        <button onclick={tapA}>Tap “a”</button>
        <button onclick={winShiftS}>Win+Shift+S</button>
        <button onclick={nudgeMouse}>Nudge mouse (40,40)</button>
      </div>
    </section>
  {/if}
</main>
