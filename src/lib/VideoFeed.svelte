<script lang="ts">
  import { onMount, onDestroy } from "svelte";

  // `onpick` fires when the operator clicks the screen (to grab input).
  // `devices` / `selectedId` are bindable so the parent can render the capture
  // source picker inside its toolbar (avoids an overlay that collides with the
  // bar). VideoFeed still owns the actual MediaStream lifecycle.
  let {
    onpick,
    devices = $bindable([]),
    selectedId = $bindable(""),
  }: {
    onpick?: () => void;
    devices?: MediaDeviceInfo[];
    selectedId?: string;
  } = $props();

  let videoEl: HTMLVideoElement | undefined = $state();
  let error = $state<string | null>(null);
  let stream: MediaStream | null = null;
  // deviceId currently streaming — guards the $effect against redundant restarts.
  let activeId = "";

  async function refreshDevices() {
    const all = await navigator.mediaDevices.enumerateDevices();
    devices = all.filter((d) => d.kind === "videoinput");
  }

  async function startStream() {
    stopStream();
    error = null;
    const want = selectedId;
    try {
      stream = await navigator.mediaDevices.getUserMedia({
        audio: false,
        video: want
          ? { deviceId: { exact: want }, width: { ideal: 1920 }, height: { ideal: 1080 } }
          : { width: { ideal: 1920 }, height: { ideal: 1080 } },
      });
      if (videoEl) videoEl.srcObject = stream;
      // Surface an unplugged capture device (the track ends) instead of
      // freezing on the last frame; clearing the selection lets Retry — or a
      // re-plug via `devicechange` — pick up an available device.
      const track = stream.getVideoTracks()[0];
      if (track)
        track.onended = () => {
          error = "Capture device disconnected.";
          stopStream();
          activeId = "";
          selectedId = "";
          refreshDevices();
        };
      // Labels are only populated once capture permission is granted.
      await refreshDevices();
      const actual = track?.getSettings().deviceId ?? want ?? devices[0]?.deviceId ?? "";
      activeId = actual;
      if (selectedId !== actual) selectedId = actual;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  function stopStream() {
    stream?.getTracks().forEach((t) => t.stop());
    stream = null;
    if (videoEl) videoEl.srcObject = null;
  }

  // Keep the picker in sync with hot-plugged hardware, and auto-recover the
  // feed if a device reappears after we lost it.
  function onDeviceChange() {
    refreshDevices();
    if (!stream) startStream();
  }

  // Restart when the parent's source picker selects a different device.
  $effect(() => {
    if (selectedId && selectedId !== activeId) startStream();
  });

  onMount(() => {
    startStream();
    navigator.mediaDevices.addEventListener("devicechange", onDeviceChange);
  });
  onDestroy(() => {
    stopStream();
    navigator.mediaDevices.removeEventListener("devicechange", onDeviceChange);
  });
</script>

<div class="video-wrap" onclick={onpick} role="presentation">
  <!-- svelte-ignore a11y_media_has_caption -->
  <video bind:this={videoEl} autoplay playsinline muted></video>

  {#if error}
    <div class="video-msg">
      <p>Couldn't start capture.</p>
      <p class="mono">{error}</p>
      <button onclick={(e) => (e.stopPropagation(), startStream())}>Retry</button>
    </div>
  {:else if devices.length === 0}
    <div class="video-msg muted">Waiting for a capture device…</div>
  {/if}
</div>

<style>
  .video-wrap {
    position: absolute;
    inset: 0;
    background: #000;
    overflow: hidden;
    cursor: pointer;
  }
  video {
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
  }
  .video-msg {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    text-align: center;
    color: #e6e8ec;
    padding: 24px;
  }
</style>
