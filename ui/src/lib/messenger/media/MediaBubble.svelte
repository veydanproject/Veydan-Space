<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The attachment of one message: upload or download state, preview for
  what a webview renders passively, a file card for everything else.
  Shared by DMs and, later, groups.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { isTauriHost, mediaErrorCode, mediaOf, messengerApi, messengerError, type MessengerMessage } from '../api';
  import { bytes, fileIcon, percent } from '../shared/format';
  import { transferStore } from './transferStore.svelte';
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
  /** A blob url made here, to give back when the bubble goes. */
  let owned: string | null = null;

  /** One word for the whole component. */
  const phase = $derived.by(() => {
    if (out && (m.status === 'uploading' || live?.status === 'running' && live.direction === 'up')) return 'uploading';
    if (out && m.status === 'paused') return 'upload_paused';
    if (out && m.status === 'failed' && m.id.startsWith('local:')) return 'upload_failed';
    if (local || media?.local_path) return 'here';
    if (live?.direction === 'down' && (live.status === 'running' || live.status === 'queued')) return 'downloading';
    if (live?.direction === 'down' && live.status === 'paused') return 'download_paused';
    if (live?.direction === 'down' && live.status === 'failed') return 'download_failed';
    return 'remote';
  });

  const progress = $derived(live ? percent(live.done_bytes, live.total_bytes) : 0);
  const transferId = $derived(live?.transfer_id ?? media?.transfer_id ?? null);
  const previewable = $derived(!!media && media.kind !== "file" && media.size <= 24 * 1024 * 1024 && !previewFailed);
  const moving = $derived(phase === "uploading" || phase === "downloading");

  /** A steady colour per file for the place a picture will take. */
  const hue = $derived.by(() => {
    const s = media?.name ?? m.id;
    let h = 0;
    for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) >>> 0;
    return h % 360;
  });

  /** What pressing a tile or a card does: look or open, fetch, pause, go on. */
  function press() {
    if (phase === 'here') { if (src && (media?.kind === 'image' || media?.kind === 'video')) view(); else open(); return; }
    if (moving && transferId) { act(() => messengerApi.media.pause(transferId)); return; }
    if ((phase === 'upload_paused' || phase === 'upload_failed') && transferId) { act(() => messengerApi.media.resume(transferId)); return; }
    if (phase === 'remote' || phase === 'download_paused' || phase === 'download_failed') download(true);
  }

  const RING = 2 * Math.PI * 17;

  let circle = $state<HTMLVideoElement | null>(null);
  let circlePlaying = $state(false);
  /** The user asked for the sound; the silent first frame is not that. */
  let circleWanted = false;
  // My own circle is on the device before it is uploaded: it is shown at once.
  const circleShown = $derived(media?.kind === "circle" && !!src && (phase === "here" || phase === "uploading"));
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
    const code = mediaErrorCode(e);
    return code ? $t(`msg_media_${code.replace('.', '_')}` as 'msg_media_err_network') : messengerError(e);
  }

  const failure = $derived.by(() => {
    const raw = error || live?.failure_reason || (phase === 'upload_failed' ? m.failure_reason : null);
    if (!raw) return '';
    const code = mediaErrorCode(raw);
    return code ? $t(`msg_media_${code.replace('.', '_')}` as 'msg_media_err_network') : raw;
  });

  async function loadPreview() {
    if (!previewable || src) return;
    try {
      const url = await messengerApi.media.dataUrl(m.id);
      // Sound and video play from a blob, never from `data:` (blob-url.ts).
      if (url && media?.kind !== 'image') { owned = playableUrl(url); src = owned; }
      else src = url;
    } catch { previewFailed = true; }
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
    else if (!out && !m.id.startsWith('local:')) download(false);
    return () => { if (owned?.startsWith('blob:')) URL.revokeObjectURL(owned); };
  });
</script>

{#snippet actions()}
    {#if phase === 'uploading' || phase === 'downloading'}
      {#if transferId}
        <button class="act" onclick={() => act(() => messengerApi.media.pause(transferId))} title={$t('msg_media_pause')}><Icon name="square" size={13} /></button>
        <button class="act" onclick={() => act(() => messengerApi.media.cancel(transferId))} title={$t('msg_media_cancel')}><Icon name="x" size={14} /></button>
      {/if}
    {:else if phase === 'upload_paused' || phase === 'upload_failed'}
      {#if transferId}
        <button class="act" onclick={() => act(() => messengerApi.media.resume(transferId))} title={$t('msg_media_resume')}><Icon name="play" size={13} /></button>
        <button class="act" onclick={() => act(() => messengerApi.media.cancel(transferId))} title={$t('msg_media_cancel')}><Icon name="x" size={14} /></button>
      {/if}
    {:else if phase === 'remote' || phase === 'download_paused' || phase === 'download_failed'}
      <button class="act primary" disabled={busy} onclick={() => download(true)} title={$t('msg_media_download')}><Icon name="download" size={14} /></button>
    {:else}
      <button class="act" onclick={open} title={$t('msg_media_open')}><Icon name="external-link" size={14} /></button>
      <button class="act" onclick={saveAs} title={$t('msg_media_save_as')}><Icon name="save" size={14} /></button>
    {/if}
{/snippet}

{#if media && variant === 'card'}
  <div class="card-file" class:wide class:failed={!!failure}>
    <button class="hit" onclick={press} oncontextmenu={(e) => e.preventDefault()} title={media.name}
      aria-label={phase === 'here' ? $t('msg_media_open') : moving ? $t('msg_media_pause') : $t('msg_media_download')}></button>
    <span class="kind" class:spin={moving}><Icon name={moving ? 'loader' : fileIcon(media.name, media.mime)} size={wide ? 20 : 24} /></span>
    <span class="about">
      <span class="name">{media.name}</span>
      <span class="sub">
        {#if moving || phase === 'upload_paused' || phase === 'download_paused'}{progress}% · {bytes(media.size)}
        {:else}{bytes(media.size)}{/if}
      </span>
    </span>
    <span class="acts">{@render actions()}</span>
    {#if moving || phase === 'upload_paused' || phase === 'download_paused'}
      <span class="line-bar" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={progress}><span style="width:{progress}%"></span></span>
    {/if}
    {#if failure}<span class="card-fail" title={failure}><Icon name="alert-triangle" size={11} />{failure}</span>{/if}
  </div>
{:else if media && variant === 'tile'}
  <div class="tile kind-{media.kind}" class:natural={fit === 'natural'} class:shown={phase === 'here' && !!src} style="--h:{hue}">
    {#if phase === 'here' && src && media.kind === 'image'}
      <img {src} alt={media.name} onerror={() => { previewFailed = true; src = null; }} />
    {:else if phase === 'here' && src && media.kind === 'video'}
      <!-- svelte-ignore a11y_media_has_caption -->
      <video {src} muted playsinline preload="metadata"></video>
    {:else}
      <span class="placeholder-name">{media.name}</span>
    {/if}

    <button class="hit" onclick={press} oncontextmenu={(e) => e.preventDefault()}
      aria-label={phase === 'here' && src ? $t('msg_media_open') : moving ? $t('msg_media_pause') : $t('msg_media_download')}
      title={media.name}></button>

    {#if moving}
      <span class="center" aria-hidden="true">
        <svg viewBox="0 0 40 40" class="ring"><circle cx="20" cy="20" r="17" /><circle class="done" cx="20" cy="20" r="17" style="stroke-dasharray:{RING};stroke-dashoffset:{RING * (1 - progress / 100)}" /></svg>
        <Icon name="x" size={14} />
      </span>
    {:else if phase === 'here' && src && media.kind === 'video'}
      <span class="center" aria-hidden="true"><Icon name="play" size={20} /></span>
    {:else if phase !== 'here' || !src}
      <span class="center" aria-hidden="true">
        <Icon name={phase === 'upload_paused' || phase === 'upload_failed' ? 'upload' : 'download'} size={18} />
      </span>
      {#if phase !== 'here'}<span class="size">{phase === 'upload_paused' || phase === 'download_paused' ? `${progress}%` : bytes(media.size)}</span>{/if}
    {/if}
    {#if failure}<span class="bad" title={failure}><Icon name="alert-triangle" size={12} /></span>{/if}
  </div>
{:else if media}
  <div class="media kind-{media.kind}" class:out>
    {#if media.kind === "voice"}
      <VoicePlayer id={m.id} {src} durationMs={media.duration_ms ?? 0} waveform={media.waveform ?? []} {out} busy={busy || moving}
        onneed={() => download(true)} />
    {:else if circleShown}
      <button class="circle" onclick={toggleCircle} aria-label={circlePlaying ? $t("msg_media_pause") : $t("msg_voice_play")}>
        <!-- svelte-ignore a11y_media_has_caption -->
        <video bind:this={circle} {src} poster={NO_POSTER} playsinline preload="auto"
          onloadeddata={() => { if (circle) showFirstFrame(circle, () => circleWanted); }}
          onplay={() => (circlePlaying = !circle?.muted)} onpause={() => (circlePlaying = false)} onended={() => { circlePlaying = false; circleWanted = false; }}></video>
        {#if phase === "uploading"}<span class="circle-play spin"><Icon name="loader" size={26} /></span>
        {:else if !circlePlaying}<span class="circle-play"><Icon name="play" size={26} /></span>{/if}
      </button>
    {:else if phase === "here" && src && media.kind === "image"}
      <button class="thumb" onclick={view} title={$t("msg_media_open")}>
        <img {src} alt={media.name} onerror={() => { previewFailed = true; src = null; }} />
      </button>
    {:else if phase === 'here' && src && media.kind === 'video'}
      <!-- svelte-ignore a11y_media_has_caption -->
      <video class="player" {src} controls preload="metadata" playsinline></video>
    {:else if phase === 'here' && src && media.kind === 'audio'}
      <audio class="audio" {src} controls preload="metadata"></audio>
    {/if}

    {#if media.kind !== "voice" && !circleShown}
    <div class="file-row">
      <span class="ico" class:spin={phase === 'uploading' || phase === 'downloading'}>
        {#if phase === 'uploading' || phase === 'downloading'}<Icon name="loader" size={18} />
        {:else if phase === 'remote' || phase === 'download_paused' || phase === 'download_failed'}<Icon name="download" size={18} />
        {:else if phase === 'upload_failed'}<Icon name="alert-triangle" size={18} />
        {:else if media.kind === 'image'}<Icon name="image" size={18} />
        {:else if media.kind === 'video'}<Icon name="video" size={18} />
        {:else if media.kind === 'audio'}<Icon name="mic" size={18} />
        {:else if media.kind === "circle"}<Icon name="video" size={18} />
        {:else}<Icon name={fileIcon(media.name, media.mime)} size={18} />{/if}
      </span>
      <span class="info">
        <span class="name" title={media.name}>{media.name}</span>
        <span class="sub">
          {#if phase === 'uploading'}{$t('msg_media_uploading', { pct: String(progress), size: bytes(media.size) })}
          {:else if phase === 'downloading'}{$t('msg_media_downloading', { pct: String(progress), size: bytes(media.size) })}
          {:else if phase === 'upload_paused' || phase === 'download_paused'}{$t('msg_media_paused', { pct: String(progress) })}
          {:else}{bytes(media.size)}{/if}
        </span>
      </span>
      <span class="actions">{@render actions()}</span>
    </div>
    {/if}

    {#if phase === 'uploading' || phase === 'downloading' || phase === 'upload_paused' || phase === 'download_paused'}
      <div class="bar" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={progress}>
        <span style="width:{progress}%"></span>
      </div>
    {/if}
    {#if failure}<div class="fail">{failure}</div>{/if}
  </div>
{/if}

<style>
  .media { display: flex; flex-direction: column; gap: 6px; min-width: 240px; max-width: 360px; }
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
  .card-file .about { display: flex; flex-direction: column; gap: 1px; min-width: 0; width: 100%; }
  .card-file .name {
    font-size: var(--fs-xs); line-height: 1.3; overflow-wrap: anywhere;
    display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;
  }
  .card-file .sub { font-size: var(--fs-2xs); color: var(--text-3); }
  .card-file .acts { position: relative; z-index: 1; display: inline-flex; gap: 2px; margin-top: auto; }
  .card-file.wide { flex-direction: row; text-align: left; padding: 8px 8px 8px 10px; gap: var(--sp-2); }
  .card-file.wide .kind { width: 40px; height: 40px; border-radius: 10px; }
  .card-file.wide .about { width: auto; flex: 1 1 auto; }
  .card-file.wide .acts { flex-shrink: 0; }
  .card-file.wide .name { display: block; white-space: nowrap; text-overflow: ellipsis; font-size: var(--fs-sm); font-weight: var(--fw-semibold); }
  .card-file.wide .acts { margin: 0 0 0 auto; }
  .card-file.failed { box-shadow: inset 0 0 0 1px var(--danger-border); }
  .line-bar { position: absolute; left: 8px; right: 8px; bottom: 3px; height: 2px; border-radius: 2px; background: var(--surface-3); overflow: hidden; pointer-events: none; }
  .line-bar span { display: block; height: 100%; background: var(--accent); transition: width 0.25s var(--ease); }
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
  .tile.natural { height: auto; min-height: 120px; }
  .tile.natural img, .tile.natural video { height: auto; max-height: 360px; object-fit: contain; }
  .tile.natural:not(.shown) { aspect-ratio: 4 / 3; }
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
  .ring { position: absolute; inset: 0; transform: rotate(-90deg); }
  .ring circle { fill: none; stroke: rgba(255, 255, 255, 0.25); stroke-width: 2.5; }
  .ring circle.done { stroke: #fff; transition: stroke-dashoffset 0.25s var(--ease); }
  .size {
    position: absolute; left: 50%; top: calc(50% + 24px); transform: translateX(-50%); font-size: 10px; color: #fff;
    background: rgba(0, 0, 0, 0.45); padding: 1px 6px; border-radius: var(--radius-pill); pointer-events: none; white-space: nowrap;
  }
  .bad {
    position: absolute; top: 6px; right: 6px; width: 20px; height: 20px; border-radius: 50%; display: flex; align-items: center;
    justify-content: center; background: var(--danger-bg); color: var(--danger-text); pointer-events: none;
  }
  .thumb { border: none; padding: 0; background: none; cursor: zoom-in; border-radius: 10px; overflow: hidden; display: block; }
  .thumb img { display: block; max-width: 100%; max-height: 320px; object-fit: contain; border-radius: 10px; background: var(--surface-3); }
  .circle { position: relative; width: 220px; height: 220px; border: none; padding: 0; border-radius: 50%; overflow: hidden; background: #000; cursor: pointer; }
  .circle video { width: 100%; height: 100%; object-fit: cover; display: block; }
  .circle-play { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; color: #fff; background: rgba(0, 0, 0, 0.28); }
  .circle-play.spin :global(svg) { animation: spin 1.1s linear infinite; }
  .player { max-width: 100%; max-height: 320px; border-radius: 10px; background: #000; }
  .audio { width: 100%; height: 36px; }
  .file-row { display: flex; align-items: center; gap: var(--sp-2); }
  .ico {
    width: 38px; height: 38px; flex-shrink: 0; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center;
    background: var(--surface-3); color: var(--accent-text-2);
  }
  .out .ico { background: color-mix(in srgb, var(--accent) 16%, transparent); }
  .spin :global(svg) { animation: spin 1.1s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .info { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-semibold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { font-size: var(--fs-2xs); color: var(--text-3); }
  .actions { display: inline-flex; gap: 2px; flex-shrink: 0; }
  .act { border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; padding: 6px; border-radius: var(--radius-sm); }
  .act:hover:not(:disabled) { color: var(--text); background: var(--surface-3); }
  .act.primary { color: var(--accent-text-2); }
  .act:disabled { opacity: 0.4; cursor: default; }
  .bar { height: 3px; border-radius: 2px; background: var(--surface-3); overflow: hidden; }
  .bar span { display: block; height: 100%; background: var(--accent); transition: width 0.25s var(--ease); }
  .fail { font-size: var(--fs-2xs); color: var(--danger-text); }
</style>
