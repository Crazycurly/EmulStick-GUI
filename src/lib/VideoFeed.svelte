<script lang="ts">
  import { onMount, onDestroy } from "svelte";

  // `onpick` fires when the operator clicks the screen (to grab input).
  let { onpick }: { onpick?: () => void } = $props();

  let videoEl: HTMLVideoElement | undefined = $state();
  let devices = $state<MediaDeviceInfo[]>([]);
  let selectedId = $state<string>("");
  let error = $state<string | null>(null);
  let stream: MediaStream | null = null;

  async function startStream() {
    stopStream();
    error = null;
    try {
      stream = await navigator.mediaDevices.getUserMedia({
        audio: false,
        video: selectedId
          ? { deviceId: { exact: selectedId }, width: { ideal: 1920 }, height: { ideal: 1080 } }
          : { width: { ideal: 1920 }, height: { ideal: 1080 } },
      });
      if (videoEl) videoEl.srcObject = stream;
      // Labels are only populated once capture permission is granted.
      const all = await navigator.mediaDevices.enumerateDevices();
      devices = all.filter((d) => d.kind === "videoinput");
      if (!selectedId) {
        selectedId =
          stream.getVideoTracks()[0]?.getSettings().deviceId ??
          devices[0]?.deviceId ??
          "";
      }
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  function stopStream() {
    stream?.getTracks().forEach((t) => t.stop());
    stream = null;
    if (videoEl) videoEl.srcObject = null;
  }

  function onDeviceChange(e: Event) {
    selectedId = (e.currentTarget as HTMLSelectElement).value;
    startStream();
  }

  onMount(startStream);
  onDestroy(stopStream);
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

  {#if devices.length > 1}
    <select
      class="source-select"
      value={selectedId}
      onchange={onDeviceChange}
      onclick={(e) => e.stopPropagation()}
    >
      {#each devices as d (d.deviceId)}
        <option value={d.deviceId}>{d.label || "Capture device"}</option>
      {/each}
    </select>
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
  .source-select {
    position: absolute;
    top: 10px;
    right: 12px;
    background: rgba(20, 22, 28, 0.8);
    color: #e6e8ec;
    border: 1px solid #2a2f3a;
    border-radius: 8px;
    padding: 4px 8px;
    font-size: 12px;
    max-width: 220px;
  }
</style>
