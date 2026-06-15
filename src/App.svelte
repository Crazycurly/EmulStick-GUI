<script lang="ts">
  import { onMount } from "svelte";
  import { fade } from "svelte/transition";
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
  // OS of the target system being controlled. The backend swaps Alt↔Win on
  // forwarded keys when this differs from the host OS, so modifiers line up
  // across a Mac/PC keyboard mismatch. Persisted like passthrough.
  let targetMac = $state(false);
  let locked = $state(false);
  let scanning = $state(false);
  let lastError = $state<string | null>(null);
  let needsAccessibility = $state(false);
  let reconnecting = $state(false);
  let savedDevice = $state<SavedDevice | null>(null);
  let view = $state<"compact" | "kvm">("compact");

  // HDMI capture source list/selection — owned here so the KVM toolbar can
  // render the picker; VideoFeed drives the actual stream via these binds.
  let videoDevices = $state<MediaDeviceInfo[]>([]);
  let videoSelectedId = $state("");

  // While grabbed in screen mode the toolbar hides for a clean view; a brief
  // "Ctrl+Alt to release" reminder fades in on grab so the exit is never hidden.
  let showReleaseHint = $state(false);
  // The toolbar lingers briefly after grabbing, then slides away (not abruptly).
  let barHidden = $state(false);

  const connected = $derived(connection === "Connected");
  const deviceName = $derived(savedDevice?.name ?? info?.model ?? "EmulStick");
  // Locking with neither channel enabled captures nothing — block it. (Exiting
  // lock must always stay allowed, so this only gates *entering*.)
  const canGrab = $derived(passthrough.keyboard || passthrough.mouse);

  // The lock/escape hotkey is Ctrl+Alt; on a Mac keyboard those keys are
  // labelled Control + Option, so show platform-native names.
  const isMac = navigator.userAgent.includes("Mac");
  const hotkey = isMac ? "Control+Option" : "Ctrl+Alt";

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
      events.onLockState((active) => {
        locked = active;
        if (active) needsAccessibility = false;
      }),
      events.onKeyboardLeds((l) => (leds = l)),
      events.onError((e) => {
        // The grab thread couldn't install the hook. On macOS that's almost
        // always a missing Accessibility grant, so route it to the onboarding
        // card instead of a raw error line (plan §5). Other platforms have no
        // such grant (Windows/Linux hooks need no permission) — the macOS card
        // would be misleading, so show the backend's actual reason there.
        if (e.code === "input_grab_failed") {
          if (isMac) {
            needsAccessibility = true;
            return;
          }
          lastError = humanize(e.message);
          return;
        }
        lastError = humanize(e.message);
      }),
    ];

    // When the operator returns from System Settings (window regains focus),
    // re-check the Accessibility grant so the onboarding card clears itself
    // without a manual "Re-check" click.
    const onFocus = () => {
      if (needsAccessibility) recheckAccessibility();
    };
    window.addEventListener("focus", onFocus);

    (async () => {
      store = await load("emulstick.json");
      savedDevice = (await store.get<SavedDevice>("lastDevice")) ?? null;
      // Restore the operator's channel choices. Lock stays off on startup, so
      // nothing is grabbed until they explicitly lock — restoring the flags is
      // safe and saves re-toggling them every launch. Fall back to the
      // backend's current flags if none were persisted.
      const savedPass = await store.get<PassthroughFlags>("passthrough");
      if (savedPass) {
        passthrough = savedPass;
        await commands.setPassthrough(savedPass);
      } else {
        passthrough = await commands.getPassthrough();
      }
      // Restore the target-system OS choice and mirror it to the backend.
      targetMac = (await store.get<boolean>("targetMac")) ?? false;
      await commands.setTargetOs(targetMac);
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
      window.removeEventListener("focus", onFocus);
    };
  });

  // Map known backend error strings to operator-friendly text; pass others
  // through unchanged so nothing is hidden.
  function humanize(msg: string): string {
    if (/no bluetooth adapter/i.test(msg))
      return "No Bluetooth adapter found — make sure Bluetooth is turned on.";
    if (/timed out/i.test(msg))
      return `${msg} Check the device is powered on and in range.`;
    if (/not found/i.test(msg))
      return "Device not found — is it powered on and in range?";
    return msg;
  }

  async function run(action: () => Promise<unknown>) {
    lastError = null;
    try {
      await action();
    } catch (e) {
      lastError = humanize(String(e));
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

  // Gate lock entry on the macOS Accessibility grant — without it the global
  // input hook silently fails (plan §5). `checkAccessibility(true)` also pops
  // macOS's grant dialog the first time and adds the app to the list.
  async function beginLock() {
    const ok = await commands.checkAccessibility(true);
    if (!ok) {
      needsAccessibility = true;
      return;
    }
    needsAccessibility = false;
    await commands.enterLock();
  }

  const toggleLock = () => run(locked ? commands.exitLock : beginLock);

  const openAxSettings = () => run(commands.openAccessibilitySettings);

  // Re-check after the operator grants the permission in System Settings.
  async function recheckAccessibility() {
    const ok = await commands.checkAccessibility(false);
    needsAccessibility = !ok;
    if (ok) lastError = null;
  }

  async function setFlag(key: keyof PassthroughFlags, value: boolean) {
    passthrough = { ...passthrough, [key]: value };
    await run(() => commands.setPassthrough(passthrough));
    // Persist so the choice survives a restart (see the restore in onMount).
    await store?.set("passthrough", passthrough);
    await store?.save();
  }

  // Switch the target system's OS (key mapping) and persist the choice.
  async function setTarget(mac: boolean) {
    targetMac = mac;
    await run(() => commands.setTargetOs(mac));
    await store?.set("targetMac", mac);
    await store?.save();
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
    if (!locked && canGrab) toggleLock();
  }

  // On grab: flash the "Ctrl+Alt to release" reminder, and slide the toolbar
  // away after a short linger (not the instant you grab). On release, both
  // revert immediately so the bar snaps back.
  $effect(() => {
    if (!locked) {
      showReleaseHint = false;
      barHidden = false;
      return;
    }
    showReleaseHint = true;
    const hideHint = setTimeout(() => (showReleaseHint = false), 2600);
    const hideBar = setTimeout(() => (barHidden = true), 1200);
    return () => {
      clearTimeout(hideHint);
      clearTimeout(hideBar);
    };
  });
</script>

<!-- OS glyphs as inline SVG: the 🪟 "window" emoji renders as tofu on Windows
     itself (and varies elsewhere), so the picker draws crisp currentColor marks
     that inherit the surrounding text colour. -->
{#snippet osIcon(mac: boolean)}
  {#if mac}
    <svg class="os-ico" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path d="M12.152 6.896c-.948 0-2.415-1.078-3.96-1.04-2.04.027-3.91 1.183-4.961 3.014-2.117 3.675-.546 9.103 1.519 12.09 1.013 1.454 2.208 3.09 3.792 3.039 1.52-.065 2.09-.987 3.935-.987 1.831 0 2.35.987 3.96.948 1.637-.026 2.676-1.48 3.676-2.948 1.156-1.688 1.636-3.325 1.662-3.415-.039-.013-3.182-1.221-3.22-4.857-.026-3.04 2.48-4.494 2.597-4.559-1.429-2.09-3.623-2.324-4.39-2.376-2-.156-3.675 1.09-4.61 1.09zm3.378-3.066c.843-1.012 1.4-2.427 1.245-3.83-1.207.052-2.662.805-3.532 1.818-.78.896-1.454 2.338-1.273 3.714 1.338.104 2.715-.688 3.559-1.701"/>
    </svg>
  {:else}
    <svg class="os-ico" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path d="M3 3h8v8H3zM13 3h8v8h-8zM3 13h8v8H3zM13 13h8v8h-8z"/>
    </svg>
  {/if}
{/snippet}

{#if view === "kvm"}
  <!-- ── PiKVM-style screen mode ──────────────────────────────────────── -->
  <div class="kvm">
    <VideoFeed bind:devices={videoDevices} bind:selectedId={videoSelectedId} onpick={grabFromScreen} {locked} />

    <div class="kvm-bar" class:hidden={barHidden}>
      <button class="kvm-btn" onclick={exitKvm} title="Back to compact">‹ Back</button>
      <span class="kvm-status" class:connected class:locked class:reconnecting>
        <span class="dot"></span>
        {locked ? "Locked" : reconnecting ? "Reconnecting…" : connection}
      </span>

      <div class="spacer"></div>

      {#if videoDevices.length > 1}
        <select class="kvm-source" bind:value={videoSelectedId} title="HDMI capture source" aria-label="Capture source">
          {#each videoDevices as d (d.deviceId)}
            <option value={d.deviceId}>{d.label || "Capture device"}</option>
          {/each}
        </select>
      {/if}

      <button class="kvm-chip" class:on={passthrough.keyboard} aria-pressed={passthrough.keyboard} title="Toggle keyboard passthrough" onclick={() => setFlag("keyboard", !passthrough.keyboard)}><span class="dot"></span>⌨️ Keyboard</button>
      <button class="kvm-chip" class:on={passthrough.mouse} aria-pressed={passthrough.mouse} title="Toggle mouse passthrough" onclick={() => setFlag("mouse", !passthrough.mouse)}><span class="dot"></span>🖱️ Mouse</button>
      <button class="kvm-chip" title="Target system OS — when it differs from this computer, Alt/⌘/Win are remapped so the modifiers line up" onclick={() => setTarget(!targetMac)}>{@render osIcon(targetMac)}{targetMac ? "macOS" : "Windows"}</button>

      <button
        class="kvm-grab"
        class:locked
        disabled={!locked && !canGrab}
        title={!locked && !canGrab ? "Enable Keyboard or Mouse first" : ""}
        onclick={toggleLock}
      >
        {locked ? "Unlock" : "Lock"}
      </button>
    </div>

    {#if lastError}
      <div class="kvm-error" role="alert">
        <span>{lastError}</span>
        <button class="error-x" onclick={() => (lastError = null)} aria-label="Dismiss error">✕</button>
      </div>
    {/if}

    {#if needsAccessibility}
      <div class="kvm-hint onboard-hint">
        Accessibility access needed —
        <button class="link" onclick={openAxSettings}>Open Settings</button>
        to capture input
      </div>
    {:else if !locked && !canGrab}
      <div class="kvm-hint">Enable Keyboard or Mouse to capture input</div>
    {:else if !locked}
      <div class="kvm-hint">Click the screen to lock keyboard &amp; mouse · {hotkey} to unlock</div>
    {:else if showReleaseHint}
      <div class="kvm-hint release-hint" transition:fade={{ duration: 400 }}>🔒 {hotkey} to unlock</div>
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
      <div class="error" role="alert">
        <span>{lastError}</span>
        <button class="error-x" onclick={() => (lastError = null)} aria-label="Dismiss error">✕</button>
      </div>
    {/if}

    {#if needsAccessibility}
      <section class="card onboard" role="alert">
        <h2>Accessibility access needed</h2>
        <p class="muted">
          To capture keyboard &amp; mouse system-wide, allow <strong>EmulStick</strong>
          under <strong>System Settings → Privacy &amp; Security → Accessibility</strong>,
          then relaunch the app.
        </p>
        <div class="row onboard-actions">
          <button onclick={openAxSettings}>Open Settings</button>
          <button class="secondary" onclick={recheckAccessibility}>Re-check</button>
        </div>
      </section>
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
        <button class="chip big" class:on={passthrough.keyboard} aria-pressed={passthrough.keyboard} onclick={() => setFlag("keyboard", !passthrough.keyboard)}><span class="dot"></span>⌨️ Keyboard</button>
        <button class="chip big" class:on={passthrough.mouse} aria-pressed={passthrough.mouse} onclick={() => setFlag("mouse", !passthrough.mouse)}><span class="dot"></span>🖱️ Mouse</button>
      </div>

      <div class="target" role="group" aria-label="Target system OS">
        <span class="target-label" title="The OS of the system you're controlling. When it differs from this computer, Alt/⌘/Win are remapped so the modifiers line up.">Target system</span>
        <div class="seg">
          <button class:sel={!targetMac} aria-pressed={!targetMac} onclick={() => setTarget(false)}>{@render osIcon(false)}Windows</button>
          <button class:sel={targetMac} aria-pressed={targetMac} onclick={() => setTarget(true)}>{@render osIcon(true)}macOS</button>
        </div>
      </div>

      <button
        class="lock-btn"
        class:locked
        disabled={!locked && !canGrab}
        title={!locked && !canGrab ? "Enable Keyboard or Mouse first" : ""}
        onclick={toggleLock}
      >
        {locked ? `🔒 Exit lock · ${hotkey}` : !canGrab ? "Enable a channel to lock" : "🔓 Enter lock"}
      </button>

      <button class="screen-btn" onclick={enterKvm}>🖥️ Open screen</button>
      <button class="screen-btn disconnect" onclick={disconnect}>🔌 Disconnect</button>
    {/if}
  </main>
{/if}
