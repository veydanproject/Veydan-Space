<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The file of a post or a comment. A picture shows its small copy from the
  post at once and its whole self once fetched from the hosts of the
  channel (checked against its hash by the runtime) and handed back as a
  `data:` URL (`channel_media_url`); a tap opens it in the messenger's
  viewer. A video is its frame with a play badge and its length (the
  preview its sender took; one sent without it and on this device gets a
  frame taken here, else a plain tile — never the player's black box):
  a tap fetches it if need be and plays it in place (in an album's cell,
  in the viewer) from the messenger's server of files, else from a
  `blob:`; one this webview cannot decode goes back to its frame and says
  where the file is kept. A sound is a line until fetched on a tap, then
  plays in place; one on this device already (mine) plays at once.
  Any other file is a line with its name and size, fetched on a tap and
  kept on the device. A voice message is the messenger's player (its
  outline, its speed), fetched when it is played; a video circle is round,
  fetched when it comes near the screen and played on a tap (CircleNote).
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { tick, untrack } from 'svelte';
  import { locale, localeTag, t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { copied } from '$lib/core/ui/toast.svelte';
  import { TransferStatus, VoicePlayer, bytes, playableUrl, viewer, type TransferAction } from '$lib/messenger/seams/shared';
  import { channelsApi, errorText, mediaOf, type FeedItemView } from './api';
  import CircleNote from './CircleNote.svelte';
  import { channelStore } from './channelStore.svelte';
  import { channelError } from './errors';
  import { mediaCache, mediaKey, nearView } from './mediaCache';
  import { frameBox, frames, grabFrame, videoLength } from './video';

  interface Props {
    item: FeedItemView;
    /** A cell of an album's mosaic: the picture fills it, its caption is the album's. */
    tile?: boolean;
    /** A file picked and not sent yet (the post editor's preview): drawn from the tray's picture of it, nothing fetched, nothing to tap. */
    picked?: { src: string | null } | null;
  }
  let { item, tile = false, picked = null }: Props = $props();

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);

  const m = $derived(mediaOf(item));
  const picture = $derived(m?.kind === 'image');
  /** Shown in place once here: a page plays these from a `data:` URL. */
  const playable = $derived(m?.kind === 'video' || m?.kind === 'circle' ? 'video' : m?.kind === 'audio' || m?.kind === 'voice' ? 'audio' : null);
  const thumb = $derived(m?.thumb ? `data:image/jpeg;base64,${m.thumb}` : null);
  const ratio = $derived(m?.dim && m.dim[0] > 0 && m.dim[1] > 0 ? `${m.dim[0]} / ${m.dim[1]}` : '4 / 3');

  /** The file by its hash: what this device fetched of it already is shown at once, never fetched again (mediaCache). */
  const key = $derived(mediaKey(item, m));
  const keptOf = (k: string) => mediaCache.get(k) ?? null;
  const first = initial();
  function initial() {
    const e = keptOf(mediaKey(item, mediaOf(item)));
    return { path: e?.path ?? null, src: e && !e.large ? e.src : null, unshown: !!e && !e.large && !e.src };
  }

  let src = $state<string | null>(first.src);
  /** Where the file is on this device, once fetched. */
  let path = $state<string | null>(first.path);
  let busy = $state(false);
  let error = $state('');
  /** The fetched file cannot be shown in place (too large, or a type a page does not show): it is kept on the device. */
  let unshown = $state(first.unshown);
  /** The picture is near the screen: it is fetched now, not with the whole feed. */
  let near = $state(false);

  // Another file in this place (an edit, a list read again): what is kept of it, or nothing yet.
  let shownKey = untrack(() => key);
  $effect(() => {
    const k = key;
    if (k === shownKey) return;
    shownKey = k;
    untrack(() => {
      const e = keptOf(k);
      path = e?.path ?? null; src = e && !e.large ? e.src : null; unshown = !!e && !e.large && !e.src; error = '';
    });
  });

  async function fetchIt(): Promise<boolean> {
    if (path && (src || unshown || !(picture || playable))) return true;
    const k = key;
    const show = picture || !!playable;
    const { community_id: cid, id } = item;
    busy = true; error = '';
    try {
      const e = await mediaCache.fetch(k, async () => {
        const at = await channelsApi.mediaFetch(cid, id);
        let url: string | null = null;
        // A video or a sound plays from the messenger's server of files, however large, as in a chat;
        // a picture, or without that server, from a `data:` URL.
        if (playable) {
          try { url = await channelsApi.mediaPlay(cid, id); } catch { url = null; }
        }
        if (show && !url) {
          try { url = await channelsApi.mediaUrl(cid, id); } catch { url = null; }
        }
        return { path: at, src: url };
      });
      if (k !== key) return true;
      path = e.path;
      if (show) { src = e.src; unshown = !e.src; }
      return true;
    } catch (e) {
      // Paused or given up on the line of the transfer: it says so itself.
      if (k === key && !errorText(e).includes(FETCH_STOPPED)) error = channelError(e, tr);
      return false;
    } finally {
      busy = false;
    }
  }

  // A picture is fetched when it comes near the screen: its small copy stands in meanwhile. So is a circle, small as it is.
  $effect(() => {
    if ((picture || m?.kind === 'circle') && near && !src && !unshown && !busy && !error) void fetchIt();
  });

  // A video or a sound on this device already (one of mine, where it was sent from; one fetched before)
  // plays at once, as in a chat: the runtime names it only when it is here, nothing is fetched.
  let askedHere = '';
  $effect(() => {
    if (!playable || !near || src || busy || askedHere === key) return;
    const k = (askedHere = key);
    channelsApi.mediaPlay(item.community_id, item.id).then((url) => { if (url && k === key && !src) src = url; }, () => {});
  });

  // Sound and video play from `blob:`, never from `data:` (the page's `media-src`): a `data:` URL, when the
  // messenger's server of files gave none, is turned into one while shown.
  let blobUrl = $state<string | null>(null);
  $effect(() => {
    const s = playable && src?.startsWith('data:') ? src : null;
    if (!s) { blobUrl = null; return; }
    const url = playableUrl(s);
    blobUrl = url;
    return () => { if (url.startsWith('blob:')) URL.revokeObjectURL(url); };
  });
  const playSrc = $derived(blobUrl ?? src);

  // ── A video: its frame until it plays ──
  /** It plays in place (a tap on its frame). */
  let playing = $state(false);
  /** This webview could not play it (no decoder of its codec, a file it cannot read): its frame stays. */
  let cannotPlay = $state(false);
  /** The `data:` copy was tried after the server of files failed. */
  let triedCopy = false;
  /** A frame taken here, for a video sent without one. */
  let taken = $state(untrack(() => frames.peek(key) ?? null));
  /** How long it lasts as the player told it. */
  let playedMs = $state<number | null>(null);
  const box = $derived(frameBox(m?.dim));
  const posterSrc = $derived(thumb ?? taken?.src ?? null);
  const lengthMs = $derived(m?.duration_ms ?? taken?.durationMs ?? playedMs);
  /** The badge of the frame: its length, else, while it is not here, its size. */
  const badge = $derived(lengthMs ? videoLength(lengthMs) : !src && !path && m ? bytes(m.size, localeTag($locale)) : null);

  // A video on this device sent without a frame: one is taken once, when it comes near the screen.
  $effect(() => {
    if (playable !== 'video' || thumb || taken || cannotPlay || !near || !playSrc || playSrc.startsWith('data:')) return;
    const k = key;
    const url = playSrc;
    void frames.take(k, () => grabFrame(url)).then((f) => { if (f && k === key) taken = f; });
  });

  async function playVideo() {
    if (!m || busy) return;
    if (!src && !(await fetchIt())) return;
    if (!src) return;
    cannotPlay = false;
    await tick();
    // A cell of an album is too small to watch in: the viewer shows it.
    if (tile && playSrc) viewer.open({ messageId: item.id, kind: 'video', src: playSrc, name: m.name });
    else playing = true;
  }

  /** The player gave up: the `data:` copy once (small enough), else back to the frame. */
  async function videoFailed() {
    if (!triedCopy && src && !src.startsWith('data:')) {
      triedCopy = true;
      try {
        const url = await channelsApi.mediaUrl(item.community_id, item.id);
        if (url) { src = url; return; }
      } catch { /* the frame below */ }
    }
    playing = false;
    cannotPlay = true;
    // Where it is kept, for another player: it is on this device (it was played from it).
    if (!path) {
      const k = key;
      channelsApi.mediaFetch(item.community_id, item.id).then((p) => { if (k === key) path = p; }, () => {});
    }
  }

  function open() {
    if (!m) return;
    if (picture && src && !unshown) viewer.open({ messageId: item.id, kind: 'image', src, name: m.name });
    else void fetchIt();
  }

  /** The runtime's word for a fetch paused or given up (`channel_media_pause`, `channel_media_fetch_cancel`). */
  const FETCH_STOPPED = 'channel_fetch_stopped';
  /** The file coming down now, as `channel.media` tells: stage, speed, time left; gone once here. */
  const fetching = $derived(path ? null : (channelStore.fetches[item.id] ?? null));

  /** An action of the transfer's line on this file coming down: a resume or a retry fetches it again, on from what is on the disk. */
  function fetchAction(a: TransferAction): Promise<unknown> {
    if (a === 'pause') return channelsApi.mediaPause(item.id);
    if (a === 'cancel') return channelsApi.mediaFetchCancel(item.id);
    if (a === 'retry_now') return channelsApi.mediaWake(item.id);
    return fetchIt();
  }

  async function copyPath() {
    if (!path) return;
    try { await navigator.clipboard.writeText(path); copied(); } catch { /* the line stays */ }
  }
</script>

{#if m && picked}
  {#if picture || playable === 'video'}
    <div class="picture still" class:tile style:aspect-ratio={tile ? undefined : picture ? ratio : '16 / 9'} title={m.name}>
      {#if picked.src}<img src={picked.src} alt={m.name} draggable="false" />
      {:else if picture}<span class="ph"><Icon name="image" size={28} /></span>{/if}
      {#if playable === 'video'}<span class="still-play"><Icon name="play" size={18} /></span>{/if}
    </div>
  {:else}
    <div class="file">
      <span class="file-icon"><Icon name="file" size={20} /></span>
      <span class="file-text"><span class="file-name">{m.name}</span></span>
    </div>
  {/if}
{:else if m}
  {#if m.kind === 'voice'}
    <div class="rec" use:nearView={{ onnear: () => (near = true) }}>
      <VoicePlayer id={item.id} src={playSrc} durationMs={m.duration_ms ?? 0} waveform={m.waveform ?? []} out={false} busy={busy || !!fetching} onneed={() => void fetchIt()} />
      {#if fetching}<TransferStatus p={fetching} look="compact" name={m.name} size={m.size} act={fetchAction} ondownload={() => void fetchIt()} {busy} />{/if}
    </div>
  {:else if m.kind === 'circle'}
    <div class="rec" use:nearView={{ onnear: () => (near = true) }}>
      <CircleNote id={item.id} src={playSrc} durationMs={m.duration_ms ?? 0} {thumb} busy={busy || !!fetching} onneed={() => void fetchIt()} />
      {#if fetching}<TransferStatus p={fetching} look="compact" name={m.name} size={m.size} act={fetchAction} ondownload={() => void fetchIt()} {busy} />{/if}
    </div>
  {:else if picture}
    <button class="picture" class:tile style:aspect-ratio={tile ? undefined : ratio} onclick={open} aria-label={m.name} title={m.name} use:nearView={{ onnear: () => (near = true) }}>
      {#if src && !unshown}
        <img src={src} alt={m.caption ?? m.name} decoding="async" onerror={() => (unshown = true)} />
      {:else if thumb}
        <img class="thumb" src={thumb} alt={m.caption ?? m.name} decoding="async" />
      {:else}
        <span class="ph"><Icon name="image" size={28} /></span>
      {/if}
      {#if busy}<span class="busy"><Icon name="loader" size={18} /></span>{/if}
    </button>
    {#if unshown && path}
      <button class="saved" onclick={copyPath} title={$t('chan_media_copy_path')}><Icon name="check" size={12} />{$t('chan_media_saved', { path })}</button>
    {/if}
  {:else if playable === 'video' && !unshown}
    <div class="vframe" class:tile style:aspect-ratio={tile ? undefined : box.ratio} style:width={tile ? undefined : `min(${box.width}px, 100%)`}
      use:nearView={{ onnear: () => (near = true) }}>
      {#if playing && playSrc && !cannotPlay}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video class="player" controls autoplay playsinline preload="metadata" src={playSrc} poster={posterSrc ?? undefined} title={m.name}
          onloadedmetadata={(e) => { const d = e.currentTarget.duration; if (Number.isFinite(d) && d > 0) playedMs = Math.round(d * 1000); }}
          onerror={() => void videoFailed()}></video>
      {:else}
        <button class="cover" onclick={() => void playVideo()} aria-label={m.name} title={m.name} disabled={!!fetching}>
          {#if posterSrc}<img src={posterSrc} alt={m.caption ?? m.name} decoding="async" draggable="false" />
          {:else}<span class="ph"><Icon name="video" size={28} /></span>{/if}
          <span class="badge-play" class:off={cannotPlay} class:spin={busy && !fetching}>
            <Icon name={busy && !fetching ? 'loader' : cannotPlay ? 'ban' : 'play'} size={tile ? 16 : 20} />
          </span>
          {#if badge}<span class="badge-len">{badge}</span>{/if}
        </button>
      {/if}
    </div>
    {#if fetching && !tile}
      <!-- The line of a chat's transfer: stage, speed, time left, pause, resume, cancel. -->
      <div class="vline"><TransferStatus p={fetching} look="full" name={m.name} size={m.size} act={fetchAction} ondownload={() => void playVideo()} {busy} /></div>
    {/if}
    {#if cannotPlay && path && !tile}
      <button class="saved" onclick={copyPath} title={$t('chan_media_copy_path')}><Icon name="check" size={12} />{$t('chan_media_saved', { path })}</button>
    {/if}
  {:else if playable === 'audio' && src}
    <audio class="audio" controls preload="metadata" src={playSrc} title={m.name}></audio>
  {:else}
    <div class="file" use:nearView={{ onnear: () => (near = true) }}>
      <!-- A video shows the frame its sender took, as in a chat. -->
      <span class="file-icon" class:framed={thumb && playable === 'video'}>
        {#if thumb && playable === 'video'}<img src={thumb} alt="" /><span class="play"><Icon name="play" size={14} /></span>
        {:else}<Icon name={m.kind === 'video' ? 'video' : m.kind === 'image' ? 'image' : 'file'} size={20} />{/if}
      </span>
      <span class="file-text">
        <span class="file-name">{m.name}</span>
        {#if fetching}
          <!-- The line of a chat's transfer: stage, speed, time left, pause, resume, cancel. -->
          <TransferStatus p={fetching} look="full" name={m.name} size={m.size} act={fetchAction} ondownload={() => void fetchIt()} {busy} />
        {:else}
          <span class="file-size">{bytes(m.size, localeTag($locale))}</span>
        {/if}
      </span>
      {#if path}
        <button class="btn btn-ghost btn-sm" onclick={copyPath} title={path}><Icon name="check" size={13} />{$t('chan_media_saved_short')}</button>
      {:else if !fetching}
        <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => fetchIt()}>
          <Icon name={busy ? 'loader' : 'download'} size={13} />{$t('chan_media_download')}
        </button>
      {/if}
    </div>
  {/if}
  <!-- The caption is the post's words (the runtime gives it as the item's text, shown above); an envelope of before that says it here. -->
  {#if m.caption && !tile && !item.text}<p class="caption">{m.caption}</p>{/if}
  {#if error}<p class="error">{error}</p>{/if}
{/if}

<style>
  .picture {
    position: relative; display: block; width: min(420px, 100%); max-height: 420px; padding: 0; border: none; cursor: zoom-in;
    border-radius: var(--radius-md); overflow: hidden; background: var(--surface-3);
  }
  .picture img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .picture.tile { width: 100%; height: 100%; max-height: none; border-radius: 0; }
  .picture.still { cursor: default; }
  .still-play {
    position: absolute; left: 50%; top: 50%; transform: translate(-50%, -50%); width: 40px; height: 40px; border-radius: 50%;
    display: inline-flex; align-items: center; justify-content: center; background: rgba(0, 0, 0, 0.5); color: #fff;
  }
  .picture img.thumb { filter: blur(6px); transform: scale(1.05); }
  /* A phone draws a blur of every picture of a feed at a cost: the small copy stretched is soft enough. */
  @media (pointer: coarse) { .picture img.thumb { filter: none; transform: none; } }
  .ph { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; color: var(--text-3); }
  .busy {
    position: absolute; right: 8px; bottom: 8px; width: 32px; height: 32px; border-radius: 50%; display: inline-flex;
    align-items: center; justify-content: center; background: rgba(0, 0, 0, 0.5); color: #fff;
  }
  .busy :global(svg) { animation: spin 1.1s linear infinite; }
  /* A video: its frame with a play badge and its length, then the player in its place. */
  .vframe { position: relative; display: block; max-width: 100%; border-radius: var(--radius-md); overflow: hidden; background: var(--surface-3); }
  .vframe.tile { width: 100%; height: 100%; border-radius: 0; }
  .vframe .player { display: block; width: 100%; height: 100%; object-fit: contain; background: #000; }
  .cover { position: relative; display: block; width: 100%; height: 100%; padding: 0; border: none; background: none; cursor: pointer; color: inherit; }
  .cover:disabled { cursor: default; }
  .cover img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .badge-play {
    position: absolute; left: 50%; top: 50%; transform: translate(-50%, -50%); width: 48px; height: 48px; border-radius: 50%;
    display: inline-flex; align-items: center; justify-content: center; background: rgba(0, 0, 0, 0.55); color: #fff;
    backdrop-filter: blur(4px); -webkit-backdrop-filter: blur(4px);
  }
  .tile .badge-play { width: 36px; height: 36px; }
  .badge-play :global(svg) { margin-left: 2px; }
  .badge-play.off :global(svg) { margin-left: 0; }
  .cover:hover .badge-play { background: rgba(0, 0, 0, 0.7); }
  .badge-play.spin :global(svg) { margin-left: 0; animation: spin 1.1s linear infinite; }
  .badge-len {
    position: absolute; left: 8px; bottom: 8px; padding: 2px 6px; border-radius: 6px; background: rgba(0, 0, 0, 0.55); color: #fff;
    font-size: var(--fs-2xs); font-weight: var(--fw-semibold); font-variant-numeric: tabular-nums; line-height: 1.4;
  }
  .tile .badge-len { left: 6px; bottom: 6px; }
  .vline { max-width: 420px; }
  .audio { display: block; width: min(420px, 100%); }
  .file {
    display: flex; align-items: center; gap: var(--sp-3); padding: var(--sp-2) var(--sp-3); max-width: 420px;
    border: 1px solid var(--border); border-radius: var(--radius-md); background: var(--surface-2);
  }
  .file-icon { width: 38px; height: 38px; flex-shrink: 0; border-radius: var(--radius-sm); display: inline-flex; align-items: center; justify-content: center; background: var(--accent-tint); color: var(--accent-text-2); }
  .file-icon.framed { position: relative; width: 64px; height: 48px; overflow: hidden; background: #000; color: #fff; }
  .file-icon.framed img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .play { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; background: rgba(0, 0, 0, 0.3); }
  .file-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .file-name { font-size: var(--fs-sm); font-weight: var(--fw-semibold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .file-size { font-size: var(--fs-2xs); color: var(--text-3); }
  .caption { margin: 0; font-size: var(--fs-sm); color: var(--text-body); }
  .saved {
    display: inline-flex; align-items: center; gap: 4px; align-self: flex-start; max-width: 100%; border: none; background: none; padding: 0;
    font: inherit; font-size: var(--fs-2xs); color: var(--text-3); cursor: pointer; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .error { margin: 0; font-size: var(--fs-xs); color: var(--danger-text); }
  /* A voice message or a circle, its transfer's ring beside it while it comes. */
  .rec { display: flex; align-items: center; gap: var(--sp-2); min-width: 0; max-width: 100%; }
  .rec :global(.voice) { min-width: 0; flex: 0 1 320px; }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
