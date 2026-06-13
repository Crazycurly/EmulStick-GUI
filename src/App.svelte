<script lang="ts">
  import { onMount } from "svelte";
  import type { UnlistenFn } from "@tauri-apps/api/event";
  import { commands, events } from "./lib/ipc";
  import type {
    ConnectionState,
    DeviceInfo,
    DiscoveredDevice,
    LedReport,
    PassthroughFlags,
  } from "./lib/types";

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

  const connected = $derived(connection === "Connected");

  onMount(() => {
    const unlisteners: Promise<UnlistenFn>[] = [
      events.onDevicesChanged((d) => (devices = d)),
      events.onConnectionState((s) => {
        connection = s;
        if (s === "Disconnected") {
          info = null;
        }
        scanning = s === "Scanning";
      }),
      events.onLockState((active) => (locked = active)),
      events.onKeyboardLeds((l) => (leds = l)),
      events.onError((e) => (lastError = `${e.code}: ${e.message}`)),
    ];

    commands.getPassthrough().then((p) => (passthrough = p));
    commands.getDeviceInfo().then((i) => {
      if (i) {
        info = i;
        connection = "Connected";
      }
    });

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

  const connect = (id: string) =>
    run(async () => {
      info = await commands.connect(id);
    });

  const disconnect = () => run(commands.disconnect);

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
    <span class="badge" class:connected class:locked>
      {locked ? "LOCKED · " : ""}{connection}
    </span>
  </header>

  {#if lastError}
    <p class="error" role="alert">{lastError}</p>
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
              <button onclick={() => connect(device.id)} disabled={connected}>
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
