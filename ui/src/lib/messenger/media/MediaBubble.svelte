<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The attachment of one message: upload or download state, preview for
  what a webview renders passively, a file card for everything else.
  Shared by DMs and, later, groups.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t, locale, localeTag } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { isTauriHost, mediaOf, messengerApi, type MessengerMessage, type MessengerTransferProgress, type TransferStatus as Status } from '../api';
  import { bytes, fileIcon } from '../shared/format';
  import { mediaErrorText } from './errors';
  import { bubblePhase } from './phase';
  import { transferStore } from './transferStore.svelte';
  import TransferStatus from './TransferStatus.svelte';
  import VoicePlayer from './VoicePlayer.svelte';
  import { viewer } from './viewer.svelte';
  import { playback } from './playback.svelte';
  import { NO_POSTER, playableUrl, showFirstFrame } from './blob-url';

  interface Props {
    message: MessengerMessage;
    /**
     * `bubble`: the attachment with its file line (files, recordings, a picture in a reply).
     * `tile`: only the picture, filling its place in an album; `cover` crops it to the place,
     * `natural` keeps its proportions (a picture alone).
     * `card`: a file in an album: its kind, name and size; `wide` when it has a row to itself.
     */
    variant?: 'bubble' | 'tile' | 'card';
    wide?: boolean;
    fit?: 'cover' | 'natural';
  }
  let { message: m, variant = 'bubble', fit = 'cover', wide = false }: Props = $props();

  const media = $derived(mediaOf(m));
  const out = $derived(m.direction === 'out');
  const live = $derived(transferStore.get(m.id));
  let src = $state<string | null>(null);
  let local = $state<string | null>(null);
  let busy = $state(false);
  let error = $state('');
  let previewFailed = $state(false);
  /** `src` is the file itself, read by the webview from the app's server, not a copy in memory. */
  let fromFile = false;
  /** The file could not be read that way: a copy in memory instead, when it is small enough. */
  let fileFailed = false;
  /** The preview the message carries, until the picture or the video itself is shown. */
  const thumbSrc = $derived(media?.thumb ? `data:image/jpeg;base64,${media.thumb}` : null);
  /** A blob url made here, to give back when the bubble goes. */
  let owned: string | null = null;

  /** One word for the whole component. */
  const phase = $derived(bubblePhase({ out, status: m.status, id: m.id }, !!(local || media?.local_path), live));
  const uploading = $derived(phase === 'uploading' || phase === 'upload_paused' || phase === 'upload_failed');
  const moving = $derived(phase === 'uploading' || phase === 'downloading');

  /**
   * The transfer shown: its last event, or, for an upload nothing live
   * tells of yet, what its message says. `null` when the file is here, or
   * elsewhere and nothing fetches it.
   */
  const shown = $derived.by((): MessengerTransferProgress | null => {
    if (phase === 'here' || phase === 'remote') return null;
    if (live) return live;
    if (!uploading || !media) return null;
    const status: Status = phase === 'upload_paused' ? 'paused' : phase === 'upload_failed' ? 'failed' : 'queued';
    return {
      transfer_id: media.transfer_id ?? '', message_id: m.id, chat_id: m.chat_id, direction: 'up', status,
      done_bytes: 0, total_bytes: media.size, failure_reason: m.failure_reason, local_path: media.local_path ?? null,
      stage: 'queued', chunks_done: 0, chunks_total: 0, chunk_size: 0, rate_bps: 0, eta_secs: null, retry_at_ms: null,
      attempt: 0, file_name: media.name, mime: media.mime,
    };
  });
  const failed = $derived(shown?.status === 'failed');

  // A failure told by a call is old news once the transfer goes on (an
  // automatic retry, the network back) or the file is here.
  $effect(() => {
    const status = live?.status;
    if (status === 'queued' || status === 'running' || status === 'waiting_retry' || status === 'done' || phase === 'here') error = '';
  });

  const previewable = $derived(!!media && media.kind !== "file" && media.size <= 24 * 1024 * 1024 && !previewFailed);

  /** A steady colour per file for the place a picture will take. */
  const hue = $derived.by(() => {
    const s = media?.name ?? m.id;
    let h = 0;
    for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) >>> 0;
    return h % 360;
  });

  /** The place a picture or a video alone takes before it is shown: its own proportions, within reason. */
  const ratio = $derived(media?.dim ? Math.min(1.9, Math.max(0.75, media.dim[0] / media.dim[1])) : 4 / 3);

  /** What pressing a tile or a card does: look or open, or fetch. A transfer has its own buttons. */
  function press() {
    if (phase === 'here') { if (src && (media?.kind === 'image' || media?.kind === 'video')) view(); else open(); return; }
    if (!shown) download(true);
  }

  let circle = $state<HTMLVideoElement | null>(null);
  let circlePlaying = $state(false);
  /** The user asked for the sound; the silent first frame is not that. */
  let circleWanted = false;
  // My own circle is on the device before it is uploaded: it is shown at once.
  const circleShown = $derived(media?.kind === "circle" && !!src && (phase === "here" || uploading));
  $effect(() => { if (playback.current !== m.id && circlePlaying) circle?.pause(); });

  function toggleCircle() {
    if (!circle) return;
    if (circlePlaying) { circleWanted = false; circle.pause(); return; }
    circleWanted = true;
    circle.muted = false;
    playback.current = m.id;
    circle.currentTime = circle.ended ? 0 : circle.currentTime;
    circle.play().catch(() => {});
  }

  function view() {
    if (!media || !src) return;
    if (media.kind === "image" || media.kind === "video") viewer.open({ messageId: m.id, kind: media.kind, src, name: media.name });
  }

  function explain(e: unknown): string {
    return mediaErrorText(e, (key) => $t(key));
  }

  /** A recording's failure in words: its ring alone has no room for them. */
  const recordingFailure = $derived(failed && (media?.kind === 'voice' || circleShown) ? explain(shown?.failure_reason || 'err.unknown') : '');

  async function loadPreview() {
    if (src || !media) return;
    // A picture or a video is read from the file by the webview itself:
    // however large it is, it shows at once and a video seeks.
    if (isTauriHost && !fileFailed && (media.kind === 'image' || media.kind === 'video')) {
      try {
        const url = await messengerApi.media.url(m.id);
        if (url) { fromFile = true; src = url; return; }
      } catch { /* the copy in memory below */ }
    }
    if (!previewable) return;
    try {
      const url = await messengerApi.media.dataUrl(m.id);
      // Sound and video play from a blob, never from `data:` (blob-url.ts).
      if (url && media?.kind !== 'image') { owned = playableUrl(url); src = owned; }
      else src = url;
    } catch { previewFailed = true; }
  }

  function previewBroken() {
    if (fromFile) { fromFile = false; fileFailed = true; src = null; loadPreview(); return; }
    previewFailed = true; src = null;
  }

  async function download(manual: boolean) {
    if (busy) return;
    busy = true; error = '';
    try {
      const p = await messengerApi.media.download(m.id, manual);
      if (p) { local = p; await loadPreview(); }
    } catch (e) { if (manual) error = explain(e); }
    finally { busy = false; }
  }

  async function act(fn: () => Promise<unknown>) {
    error = '';
    try { await fn(); } catch (e) { error = explain(e); }
  }

  async function open() {
    await act(() => messengerApi.media.open(m.id));
  }

  async function saveAs() {
    if (!isTauriHost || !media) return;
    await act(async () => {
      const { save } = await import('@tauri-apps/plugin-dialog');
      const dest = await save({ defaultPath: media.name });
      if (dest) await messengerApi.media.saveAs(m.id, dest);
    });
  }

  // A download that finished while we were watching.
  $effect(() => {
    if (live?.direction === 'down' && live.status === 'done' && live.local_path && !local) {
      local = live.local_path;
      loadPreview();
    }
  });

  onMount(() => {
    transferStore.hydrate(m.id).catch(() => {});
    if (media?.local_path) { local = media.local_path; loadPreview(); }
    // Mine from another device is fetched as what others send is.
    else if (!m.id.startsWith('local:')) download(false);
    return () => { if (owned?.startsWith('blob:')) URL.revokeObjectURL(owned); };
  });
</script>

{#snippet here()}
  <button class="act" onclick={open} aria-label={$t('msg_media_open')} title={$t('msg_media_open')}><Icon name="external-link" size={14} /></button>
  <button class="act" onclick={saveAs} aria-label={$t('msg_media_save_as')} title={$t('msg_media_save_as')}><Icon name="save" size={14} /></button>
{/snippet}

{#snippet status(look: 'compact' | 'full', overlay = false)}
  {#if media}
    <TransferStatus p={shown} {look} {overlay} name={media.name} size={media.size} chunks={media.chunks} messageId={m.id} {busy}
      ondownload={() => download(true)} />
  {/if}
{/snippet}

{#if media && variant === 'card'}
  <div class="card-file" class:wide class:failed={failed || !!error}>
    {#if phase === 'here' || !shown}
      <button class="hit" onclick={press} oncontextmenu={(e) => e.preventDefault()} title={media.name}
        aria-label={phase === 'here' ? $t('msg_media_open') : $t('msg_media_download')}></button>
    {/if}
    <span class="kind"><Icon name={failed ? 'alert-triangle' : fileIcon(media.name, media.mime)} size={wide ? 20 : 24} /></span>
    <span class="about">
      <span class="name">{media.name}</span>
      {#if phase === 'here'}<span class="sub">{bytes(media.size, localeTag($locale))}</span>
      {:else}<span class="status">{@render status('full')}</span>{/if}
    </span>
    {#if phase === 'here'}<span class="acts">{@render here()}</span>{/if}
    {#if error}<span class="card-fail" title={error}><Icon name="alert-triangle" size={11} />{error}</span>{/if}
  </div>
{:else if media && variant === 'tile'}
  <div class="tile kind-{media.kind}" class:natural={fit === 'natural'} class:shown={(phase === 'here' || uploading) && !!src} style="--h:{hue}; --ar:{ratio}">
    <!-- The file on this device shows while it goes up too, as it will once sent. -->
    {#if (phase === 'here' || uploading) && src && media.kind === 'image'}
      <img {src} alt={media.name} onerror={previewBroken} />
    {:else if (phase === 'here' || uploading) && src && media.kind === 'video'}
      <!-- svelte-ignore a11y_media_has_caption -->
      <!-- A frame a little after the start stands for the video; the preview the message carries until then (Android draws none by itself). -->
      <video src="{src}#t=0.1" poster={thumbSrc ?? NO_POSTER} muted playsinline preload="metadata" onerror={previewBroken}></video>
    {:else}
      {#if thumbSrc}<img class="blur" src={thumbSrc} alt="" aria-hidden="true" />{/if}
      <span class="placeholder-name">{media.name}</span>
    {/if}

    {#if phase === 'here' || !shown}
      <button class="hit" onclick={press} oncontextmenu={(e) => e.preventDefault()}
        aria-label={phase === 'here' ? $t('msg_media_open') : $t('msg_media_download')} title={media.name}></button>
    {/if}

    {#if phase === 'here' && src && media.kind === 'video'}
      <span class="center" aria-hidden="true"><Icon name="play" size={20} /></span>
    {:else if phase === 'here' && !src}
      <span class="center" aria-hidden="true"><Icon name={media.kind === 'video' ? 'video' : 'image'} size={18} /></span>
    {:else if phase !== 'here'}
      {@render status('compact', true)}
    {/if}
    {#if error}<span class="bad" title={error}><Icon name="alert-triangle" size={12} /></span>{/if}
  </div>
{:else if media}
  <div class="media kind-{media.kind}" class:out>
    {#if media.kind === "voice"}
      <div class="voice-row" class:with-status={!!shown}>
        <VoicePlayer id={m.id} {src} durationMs={media.duration_ms ?? 0} waveform={media.waveform ?? []} {out} busy={busy || moving}
          onneed={() => download(true)} />
        {#if shown}{@render status('compact')}{/if}
      </div>
    {:else if circleShown}
      <div class="circle-wrap">
        <button class="circle" onclick={toggleCircle} aria-label={circlePlaying ? $t("msg_media_pause") : $t("msg_voice_play")}>
          <!-- svelte-ignore a11y_media_has_caption -->
          <video bind:this={circle} {src} poster={NO_POSTER} playsinline preload="auto"
            onloadeddata={() => { if (circle) showFirstFrame(circle, () => circleWanted); }}
            onplay={() => (circlePlaying = !circle?.muted)} onpause={() => (circlePlaying = false)} onended={() => { circlePlaying = false; circleWanted = false; }}></video>
          {#if !circlePlaying && !shown}<span class="circle-play"><Icon name="play" size={26} /></span>{/if}
        </button>
        {#if shown}{@render status('compact', true)}{/if}
      </div>
    {:else if phase === "here" && src && media.kind === "image"}
      <button class="thumb" onclick={view} title={$t("msg_media_open")}>
        <img {src} alt={media.name} onerror={previewBroken} />
      </button>
    {:else if phase === 'here' && src && media.kind === 'video'}
      <!-- svelte-ignore a11y_media_has_caption -->
      <video class="player" {src} poster={thumbSrc ?? NO_POSTER} controls preload="metadata" playsinline></video>
    {:else if phase === 'here' && src && media.kind === 'audio'}
      <audio class="audio" {src} controls preload="metadata"></audio>
    {/if}

    {#if media.kind !== "voice" && !circleShown}
    <div class="file-row">
      <span class="ico">
        {#if phase === 'remote'}<Icon name="download" size={18} />
        {:else if failed}<Icon name="alert-triangle" size={18} />
        {:else if media.kind === 'audio'}<Icon name="mic" size={18} />
        {:else if media.kind === "circle"}<Icon name="video" size={18} />
        {:else}<Icon name={fileIcon(media.name, media.mime)} size={18} />{/if}
      </span>
      <span class="info">
        <span class="name" title={media.name}>{media.name}</span>
        {#if phase === 'here'}<span class="sub">{bytes(media.size, localeTag($locale))}</span>
        {:else}{@render status('full')}{/if}
      </span>
      {#if phase === 'here'}<span class="actions">{@render here()}</span>{/if}
    </div>
    {/if}

    {#if recordingFailure}<div class="fail">{recordingFailure}</div>{/if}
    {#if error}<div class="fail">{error}</div>{/if}
  </div>
{/if}

<style>
  .media { display: flex; flex-direction: column; gap: 6px; min-width: min(240px, 100%); max-width: 360px; }
  .media.kind-circle, .media.kind-voice { min-width: 0; }
  /* A file in an album: a small card, its kind in a tinted square, like the files of the old client. */
  .card-file {
    min-width: 0; overflow: hidden;
    position: relative; height: 100%; display: flex; flex-direction: column; align-items: center; gap: 4px; text-align: center;
    padding: 10px 8px 8px; border-radius: 10px; background: color-mix(in srgb, var(--surface-3) 70%, transparent);
  }
  .card-file .hit { z-index: 0; border-radius: inherit; }
  .card-file .kind {
    width: 46px; height: 46px; flex-shrink: 0; border-radius: 12px; display: inline-flex; align-items: center; justify-content: center;
    background: color-mix(in srgb, var(--accent) 14%, transparent); color: var(--accent-text-2);
  }
  .card-file.failed .kind { background: var(--danger-bg); color: var(--danger-text); }
  .card-file .about { display: flex; flex-direction: column; gap: 1px; min-width: 0; width: 100%; }
  .card-file .name {
    font-size: var(--fs-xs); line-height: 1.3; overflow-wrap: anywhere;
    display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;
  }
  .card-file .sub { font-size: var(--fs-2xs); color: var(--text-3); }
  .card-file .status { position: relative; z-index: 1; display: block; margin-top: 3px; text-align: left; }
  .card-file .acts { position: relative; z-index: 1; display: inline-flex; gap: 2px; margin-top: auto; }
  .card-file.wide { flex-direction: row; text-align: left; padding: 8px 8px 8px 10px; gap: var(--sp-2); }
  .card-file.wide .kind { width: 40px; height: 40px; border-radius: 10px; }
  .card-file.wide .about { width: auto; flex: 1 1 auto; }
  .card-file.wide .acts { flex-shrink: 0; }
  .card-file.wide .name { display: block; white-space: nowrap; text-overflow: ellipsis; font-size: var(--fs-sm); font-weight: var(--fw-semibold); }
  .card-file.wide .acts { margin: 0 0 0 auto; }
  .card-file.failed { box-shadow: inset 0 0 0 1px var(--danger-border); }
  .card-fail { display: flex; align-items: flex-start; gap: 3px; font-size: 10px; color: var(--danger-text); line-height: 1.3; text-align: left; overflow-wrap: anywhere; }
  /* A place in an album. Nothing here is text to select: it is a picture or the promise of one. */
  .tile {
    position: relative; width: 100%; height: 100%; overflow: hidden; background-color: hsl(var(--h) 38% 72%);
    background-image: repeating-linear-gradient(135deg, rgba(255, 255, 255, 0.16) 0 1px, transparent 1px 14px);
  }
  :global([data-theme='dark']) .tile:not(.shown) { background-color: hsl(var(--h) 24% 30%); }
  :global([data-theme='dark']) .placeholder-name { color: rgba(255, 255, 255, 0.6); }
  .tile.shown { background: var(--surface-3); }
  .tile img, .tile video { display: block; width: 100%; height: 100%; object-fit: cover; }
  .tile img.blur { position: absolute; inset: 0; filter: blur(8px); transform: scale(1.08); }
  .tile.natural { height: auto; min-height: 120px; }
  .tile.natural img, .tile.natural video { height: auto; max-height: 360px; object-fit: contain; }
  .tile.natural:not(.shown) { aspect-ratio: var(--ar, 4 / 3); }
  .placeholder-name {
    position: absolute; left: 8px; bottom: 7px; right: 8px; font-family: var(--font-mono); font-size: 10px;
    color: rgba(0, 0, 0, 0.55); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .hit { position: absolute; inset: 0; border: none; padding: 0; margin: 0; background: none; cursor: pointer; }
  .tile.shown .hit { cursor: zoom-in; }
  .center {
    position: absolute; left: 50%; top: 50%; width: 40px; height: 40px; margin: -20px 0 0 -20px; border-radius: 50%;
    display: flex; align-items: center; justify-content: center; background: rgba(0, 0, 0, 0.45); color: #fff; pointer-events: none;
  }
  .bad {
    position: absolute; top: 6px; right: 6px; width: 20px; height: 20px; border-radius: 50%; display: flex; align-items: center;
    justify-content: center; background: var(--danger-bg); color: var(--danger-text); pointer-events: none;
  }
  .thumb { border: none; padding: 0; background: none; cursor: zoom-in; border-radius: 10px; overflow: hidden; display: block; }
  .thumb img { display: block; max-width: 100%; max-height: 320px; object-fit: contain; border-radius: 10px; background: var(--surface-3); }
  .voice-row { display: flex; align-items: center; gap: var(--sp-2); min-width: 0; }
  /* The player gives up width to the status beside it: a narrow screen holds both. */
  .voice-row.with-status :global(.voice) { min-width: 0; flex: 1 1 auto; }
  .circle-wrap { position: relative; width: 220px; height: 220px; max-width: 100%; }
  .circle { position: relative; width: 220px; height: 220px; max-width: 100%; border: none; padding: 0; border-radius: 50%; overflow: hidden; background: #000; cursor: pointer; }
  .circle video { width: 100%; height: 100%; object-fit: cover; display: block; }
  .circle-play { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; color: #fff; background: rgba(0, 0, 0, 0.28); }
  .player { max-width: 100%; max-height: 320px; border-radius: 10px; background: #000; }
  .audio { width: 100%; height: 36px; }
  .file-row { display: flex; align-items: center; gap: var(--sp-2); }
  .file-row:has(.info > :global(.full)) { align-items: flex-start; }
  .ico {
    width: 38px; height: 38px; flex-shrink: 0; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center;
    background: var(--surface-3); color: var(--accent-text-2);
  }
  .out .ico { background: color-mix(in srgb, var(--accent) 16%, transparent); }
  .info { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-semibold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { font-size: var(--fs-2xs); color: var(--text-3); }
  .actions { display: inline-flex; gap: 2px; flex-shrink: 0; }
  .act { border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; padding: 6px; border-radius: var(--radius-sm); }
  .act:hover:not(:disabled) { color: var(--text); background: var(--surface-3); }
  .fail { font-size: var(--fs-2xs); color: var(--danger-text); }
</style>
