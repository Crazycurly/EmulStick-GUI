<script lang="ts">
  import { onMount } from "svelte";
  import type { UnlistenFn } from "@tauri-apps/api/event";
  import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
  import { load, type Store } from "@tauri-apps/plugin-store";
  import { commands, events } from "./lib/ipc";
  import VideoFeed from "./lib/VideoFeed.svelte";
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
  let view = $state<"compact" | "kvm">("compact");

  const connected = $derived(connection === "Connected");
  const deviceName = $derived(savedDevice?.name ?? info?.model ?? "EmulStick");

  let store: Store | null = null;
  let intentional = false;
  let reconnectToken = 0;

  const COMPACT_W = 400;
  const KVM = { w: 1280, h: 800 };
  let mainEl: HTMLElement | undefined = $state();

  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

  onMount(() => {
    const unlisteners: Promise<UnlistenFn>[] = [
      events.onDevicesChanged((d) => (devices = d)),
      events.onConnectionState((s) => {
        connection = s;
        if (s === "Connected") reconnecting = false;
        if (s === "Disconnected") {
          info = null;
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
      const di = await commands.getDeviceInfo();
      if (di) {
        info = di;
        connection = "Connected";
      } else if (savedDevice) {
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

  // Retry the saved device with bounded exponential backoff (plan §9).
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
    reconnectToken++;
    reconnecting = false;
    savedDevice = null;
    await store?.delete("lastDevice");
    await store?.save();
    await run(commands.disconnect);
    await exitKvm();
  }

  const toggleLock = () =>
    run(locked ? commands.exitLock : commands.enterLock);

  function setFlag(key: keyof PassthroughFlags, value: boolean) {
    passthrough = { ...passthrough, [key]: value };
    run(() => commands.setPassthrough(passthrough));
  }

  // Compact mode auto-fits the window to its content (no blank space) and is
  // not user-resizable; only KVM mode is a large, resizable window.
  let fitTimer: ReturnType<typeof setTimeout> | undefined;

  // Auto-size the compact window to its content. Two-pass: size to the measured
  // content height, then correct for window chrome using the webview's own
  // innerHeight (robust whether setSize means inner or outer size).
  async function fitCompact() {
    if (view !== "compact" || !mainEl) return;
    try {
      const win = getCurrentWindow();
      const content = mainEl.offsetHeight;
      await win.setResizable(true);
      await win.setSize(new LogicalSize(COMPACT_W, content));
      await new Promise((r) => setTimeout(r, 50));
      const delta = content - window.innerHeight;
      if (Math.abs(delta) > 1) {
        await win.setSize(new LogicalSize(COMPACT_W, content + delta));
      }
      await win.setResizable(false);
    } catch {
      /* window APIs unavailable outside Tauri */
    }
  }

  async function enterKvm() {
    view = "kvm";
    try {
      const win = getCurrentWindow();
      await win.setResizable(true);
      await win.setSize(new LogicalSize(KVM.w, KVM.h));
      await win.center();
    } catch {
      /* ignore */
    }
  }

  function exitKvm() {
    if (view !== "kvm") return;
    view = "compact"; // the effect below re-fits the compact window
  }

  // Re-fit whenever the compact content changes size (info load, errors, view…).
  $effect(() => {
    if (!mainEl) return;
    const schedule = () => {
      clearTimeout(fitTimer);
      fitTimer = setTimeout(fitCompact, 40);
    };
    const ro = new ResizeObserver(schedule);
    ro.observe(mainEl);
    return () => {
      ro.disconnect();
      clearTimeout(fitTimer);
    };
  });

  // Clicking the screen grabs input (PiKVM-style). Exit via the Ctrl+Alt hotkey.
  function grabFromScreen() {
    if (!locked) toggleLock();
  }
</script>

{#if view === "kvm"}
  <!-- ── PiKVM-style screen mode ──────────────────────────────────────── -->
  <div class="kvm">
    <VideoFeed onpick={grabFromScreen} />

    <div class="kvm-bar">
      <button class="ghost" onclick={exitKvm} title="Back to compact">‹ Back</button>
      <span class="badge" class:connected class:locked class:reconnecting>
        {locked ? "LOCKED" : reconnecting ? "Reconnecting…" : connection}
      </span>
      <div class="spacer"></div>
      <button class="chip" class:on={passthrough.keyboard} onclick={() => setFlag("keyboard", !passthrough.keyboard)}>⌨</button>
      <button class="chip" class:on={passthrough.mouse} onclick={() => setFlag("mouse", !passthrough.mouse)}>🖱</button>
      <button class="lock-btn small" class:danger={locked} onclick={toggleLock}>
        {locked ? "Release" : "Grab"}
      </button>
    </div>

    {#if !locked}
      <div class="kvm-hint">Click the screen to capture keyboard &amp; mouse · Ctrl+Alt to release</div>
    {/if}
  </div>
{:else}
  <!-- ── Compact mode ─────────────────────────────────────────────────── -->
  <main class="compact" bind:this={mainEl}>
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
        <span>Reconnecting to <strong>{deviceName}</strong>…</span>
        <button class="secondary" onclick={disconnect}>Stop</button>
      </div>
    {/if}

    {#if !connected && !reconnecting}
      <section class="card">
        <div class="row">
          <h2>Connect a device</h2>
          <button onclick={toggleScan}>{scanning ? "Stop" : "Scan"}</button>
        </div>
        {#if devices.length === 0}
          <p class="muted">{scanning ? "Scanning…" : "Start a scan to find your EmulStick."}</p>
        {:else}
          <ul class="devices">
            {#each devices as device (device.id)}
              <li>
                <div>
                  <strong>{device.name ?? "Unknown"}</strong>
                  {#if device.rssi != null}<span class="muted">{device.rssi} dBm</span>{/if}
                </div>
                <button onclick={() => connect(device)}>Connect</button>
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    {/if}

    {#if connected}
      <section class="card">
        <div class="row">
          <h2>{deviceName}</h2>
          <div class="leds">
            <span class="led" class:on={leds.num}>N</span>
            <span class="led" class:on={leds.caps}>C</span>
            <span class="led" class:on={leds.scroll}>S</span>
          </div>
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
      </section>

      <div class="toggles">
        <button class="chip big" class:on={passthrough.keyboard} onclick={() => setFlag("keyboard", !passthrough.keyboard)}>⌨ Keyboard</button>
        <button class="chip big" class:on={passthrough.mouse} onclick={() => setFlag("mouse", !passthrough.mouse)}>🖱 Mouse</button>
      </div>

      <button class="lock-btn" class:danger={locked} onclick={toggleLock}>
        {locked ? "🔒 Exit lock · Ctrl+Alt" : "🔓 Enter lock"}
      </button>

      <button class="screen-btn" onclick={enterKvm}>🖥 Open screen</button>
      <button class="screen-btn disconnect" onclick={disconnect}>
        <svg class="ico" viewBox="0 0 24 24" aria-hidden="true">
          <path d="M12 2.5v9" />
          <path d="M6.6 6.2a7.5 7.5 0 1 0 10.8 0" />
        </svg>
        Disconnect
      </button>
    {/if}
  </main>
{/if}
