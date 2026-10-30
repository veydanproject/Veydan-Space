<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The tab «Галерея». What it offers follows what the source of media can do (`capabilities`), not which source it is: a grid of the newest photos and videos with the tile of the camera first, or the invitation to the system's chooser. -->
<script lang="ts">
  import { onDestroy, onMount, untrack } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import type { AttachPanelProps } from '../seams/attach.svelte';
  import { mediaLibrary } from './library';
  import { CAMERA_DENIED, type Album, type MediaAccess, type MediaItem } from './MediaLibrary';
  import { CAPTURE_ID, GalleryModel, ThumbQueue, Viewer, durationLabel, reread, rowWindow } from './gallery.svelte';

  let { ctx, done, bar }: AttachPanelProps = $props();
  const library = mediaLibrary();
  const caps = library.capabilities;

  const model = new GalleryModel(library, 'media', untrack(() => ctx.limits.maxCount));
  const thumbs = new ThumbQueue((item) => library.thumbUrl(item, side * (window.devicePixelRatio || 1)));
  const viewer = new Viewer(library, thumbs);

  /** `null` while it is read; a source without a grid never asks. */
  let access = $state<MediaAccess | null>(caps.grid ? null : 'denied');
  let original = $state(false);
  let note = $state('');
  let albums = $state.raw<Album[] | null>(null);
  let choosingAlbum = $state(false);

  // ─── The access: read when the panel opens and whenever the app comes back (the settings, the system's chooser of a partial access) ───

  /** Moves with every read: an older read that ends later says nothing. */
  let reads = 0;

  /**
   * Everything read anew (`reread`), whatever the access says, the same
   * word too: a partial access may let in another set now. The viewer
   * closes at once; what is no longer there leaves the grid, and each
   * chosen item is asked of the phone by itself.
   */
  async function readAccess() {
    if (!caps.grid) return;
    const n = ++reads;
    const now = await reread(library, model, thumbs, viewer, access);
    if (n !== reads) return;
    access = now;
    albums = null;
    if (choosingAlbum) void openAlbums();
  }

  const onVisible = () => {
    if (document.visibilityState === 'visible') void readAccess();
  };
  onMount(() => {
    void readAccess();
    document.addEventListener('visibilitychange', onVisible);
  });
  onDestroy(() => document.removeEventListener('visibilitychange', onVisible));

  async function allow() {
    try {
      await library.request();
    } catch {
      return;
    }
    await readAccess();
  }

  // ─── The grid: only the rows on the screen are drawn ───

  let scrollTop = $state(0);
  let viewH = $state(0);
  let viewW = $state(0);
  const GAP = 2;
  const cols = $derived(Math.max(3, Math.floor((viewW || 360) / 120)));
  const side = $derived(Math.max(1, ((viewW || 360) - GAP * (cols - 1)) / cols));
  const rowH = $derived(side + GAP);
  /** The tiles: the camera first (when the source has one), then the items. */
  const lead = caps.capture ? 1 : 0;
  const rows = $derived(Math.ceil((model.items.length + lead) / cols));
  const win = $derived(rowWindow(scrollTop, viewH || 400, rowH, rows));
  const shown = $derived.by(() => {
    const out: { row: number; col: number; item: MediaItem | null }[] = [];
    for (let r = win.first; r <= win.last; r++) {
      for (let c = 0; c < cols; c++) {
        const i = r * cols + c - lead;
        if (i >= model.items.length) break;
        out.push({ row: r, col: c, item: i < 0 ? null : model.items[i] });
      }
    }
    return out;
  });

  // The thumbnails of what is on the screen; the next page before the end comes into view.
  $effect(() => {
    const items = shown.flatMap((s) => (s.item ? [s.item] : []));
    untrack(() => thumbs.want(items));
  });
  $effect(() => {
    if (access && access !== 'denied' && win.last >= rows - 6 && !model.done && !model.loading && !model.failed) {
      untrack(() => void model.more());
    }
  });

  // ─── The choice ───

  function toggle(item: MediaItem) {
    note = '';
    if (!model.toggle({ id: item.id, path: item.path, kind: item.kind })) note = $t('msg_attach_too_many', { n: String(model.max) });
  }

  function add() {
    if (model.chosen.length) done({ type: 'files', paths: model.paths(), original });
  }

  $effect(() => {
    const n = model.chosen.length;
    untrack(() => bar?.(n ? addBar : null));
  });

  async function capture(kind: 'photo' | 'video') {
    note = '';
    try {
      const path = await library.capture(kind);
      if (!path) return;
      // The shot goes into the choice at once; it has no tile of its own.
      if (!model.toggle({ id: `${CAPTURE_ID}${path}`, path, kind: kind === 'video' ? 'video' : 'image' })) {
        note = $t('msg_attach_too_many', { n: String(model.max) });
      }
    } catch (e) {
      const text = String(e instanceof Error ? e.message : e);
      note = text.includes(CAMERA_DENIED) ? $t('msg_attach_camera_denied') : $t('msg_attach_camera_failed');
    }
  }

  async function pickSystem() {
    const paths = await library.pickSystem({ multiple: true, kind: 'media' });
    if (paths.length) done({ type: 'files', paths: [...model.paths(), ...paths], original });
  }

  // ─── The viewer and the albums ───

  async function openAlbums() {
    choosingAlbum = true;
    if (albums) return;
    try {
      albums = await library.albums();
    } catch {
      albums = [];
    }
  }

  function chooseAlbum(id: string | undefined) {
    choosingAlbum = false;
    if (id === model.album) return;
    model.reset(id);
    scrollTop = 0;
    if (scroller) scroller.scrollTop = 0;
    void model.more();
  }

  const albumTitle = $derived(albums?.find((a) => a.id === model.album)?.title ?? $t('msg_attach_all_media'));
  let scroller = $state<HTMLDivElement>();
</script>

{#snippet circle(item: MediaItem)}
  {@const n = model.place(item.id)}
  <button
    type="button" class="pick-circle" class:on={n > 0}
    role="checkbox" aria-checked={n > 0} aria-label={n ? $t('msg_attach_chosen', { n: String(n) }) : $t('msg_attach_choose')}
    onclick={(e) => { e.stopPropagation(); toggle(item); }}
  >{#if n}{n}{/if}</button>
{/snippet}

{#snippet addBar()}
  <div class="add-bar">
    <label class="orig">
      <button type="button" class="toggle" class:on={original} role="switch" aria-checked={original} aria-label={$t('msg_xfer_original')} onclick={() => (original = !original)}></button>
      <span>{$t('msg_xfer_original')}</span>
    </label>
    <button type="button" class="m-btn-grad add" onclick={add}>{$t('msg_attach_add', { n: String(model.chosen.length) })}</button>
  </div>
{/snippet}

{#if !caps.grid || access === 'denied'}
  <!-- No grid: the source has none, or the user gave no access. -->
  <div class="empty-state gallery">
    <div class="empty-icon tile"><Icon name="image" size={28} /></div>
    {#if caps.grid}
      <p>{$t('msg_attach_no_access')}</p>
      <span>{$t('msg_attach_no_access_hint')}</span>
      <button type="button" class="m-btn-grad pick" onclick={allow}>{$t('msg_attach_allow')}</button>
      <button type="button" class="m-link" onclick={pickSystem}>{$t('msg_attach_system_gallery')}</button>
    {:else}
      <p>{$t('msg_attach_photos_videos')}</p>
      <span>{$t('msg_attach_gallery_hint')}</span>
      <button type="button" class="m-btn-grad pick" onclick={pickSystem}>{$t('msg_attach_pick_media')}</button>
    {/if}
    {#if caps.capture}
      <span class="cams">
        <button type="button" class="m-link" onclick={() => capture('photo')}><Icon name="camera" size={18} />{$t('msg_attach_camera')}</button>
        <button type="button" class="m-link" onclick={() => capture('video')}><Icon name="video" size={18} />{$t('msg_attach_camera_video')}</button>
      </span>
    {/if}
    {#if note}<span class="note" role="status">{note}</span>{/if}
  </div>
{:else}
  <div class="grid-panel">
    <div class="head">
      <button type="button" class="album" onclick={openAlbums} aria-haspopup="listbox">
        <span class="album-title">{albumTitle}</span><Icon name="chevron-down" size={16} />
      </button>
      {#if access === 'partial'}
        <span class="partial">{$t('msg_attach_partial')}</span>
        <button type="button" class="m-link more" onclick={allow}>{$t('msg_attach_more')}</button>
      {/if}
    </div>
    {#if note}<div class="note" role="status">{note}</div>{/if}

    <div class="scroll" bind:this={scroller} bind:clientHeight={viewH} bind:clientWidth={viewW} onscroll={(e) => (scrollTop = (e.currentTarget as HTMLElement).scrollTop)}>
      <div class="space" style:height="{Math.max(0, rows * rowH - GAP)}px">
        {#each shown as s (s.item?.id ?? 'camera')}
          <div class="cell" style:width="{side}px" style:height="{side}px" style:transform="translate({s.col * (side + GAP)}px, {s.row * rowH}px)">
            {#if !s.item}
              <div class="camera">
                <button type="button" class="cam-photo" onclick={() => capture('photo')}>
                  <Icon name="camera" size={22} /><span>{$t('msg_attach_camera')}</span>
                </button>
                <button type="button" class="cam-video" onclick={() => capture('video')}>
                  <Icon name="video" size={16} /><span>{$t('msg_attach_camera_video')}</span>
                </button>
              </div>
            {:else}
              {@const item = s.item}
              <button type="button" class="tile" class:chosen={model.place(item.id) > 0} onclick={() => viewer.open(item)} aria-label={item.kind === 'video' ? $t('msg_attach_video_item', { d: durationLabel(item.duration) }) : $t('msg_attach_photo_item')}>
                {#if thumbs.urls[item.id]}<img src={thumbs.urls[item.id]} alt="" draggable="false" decoding="async" onerror={() => thumbs.failed(item.id)} />{/if}
                {#if item.kind === 'video'}<span class="dur">{durationLabel(item.duration)}</span>{/if}
              </button>
              {@render circle(item)}
            {/if}
          </div>
        {/each}
      </div>
      {#if model.done && !model.items.length}
        <div class="empty-state none"><span>{$t('msg_attach_gallery_empty')}</span></div>
      {/if}
      {#if model.failed}
        <div class="empty-state none">
          <span>{$t('msg_attach_gallery_failed')}</span>
          <button type="button" class="m-link" onclick={() => model.more()}>{$t('msg_xfer_retry')}</button>
        </div>
      {/if}
    </div>

    {#if viewer.item}
      {@const item = viewer.item}
      <div class="viewer" role="dialog" aria-label={item.kind === 'video' ? $t('msg_attach_video_item', { d: durationLabel(item.duration) }) : $t('msg_attach_photo_item')}>
        {#if viewer.url}<img src={viewer.url} alt="" draggable="false" onerror={() => viewer.failed()} />{/if}
        {#if item.kind === 'video'}<span class="play"><Icon name="play" size={28} /></span><span class="dur big">{durationLabel(item.duration)}</span>{/if}
        <button type="button" class="m-ibtn back" onclick={() => viewer.close()} aria-label={$t('msg_back')}><Icon name="arrow-left" size={20} /></button>
        {@render circle(item)}
      </div>
    {/if}

    {#if choosingAlbum}
      <div class="albums" role="listbox" aria-label={$t('msg_attach_albums')}>
        <div class="m-list">
          <button type="button" class="m-row" role="option" aria-selected={!model.album} onclick={() => chooseAlbum(undefined)}>
            <span class="m-row-label">{$t('msg_attach_all_media')}</span>
            {#if !model.album}<Icon name="check" size={18} />{/if}
          </button>
          {#each albums ?? [] as a (a.id)}
            <button type="button" class="m-row" role="option" aria-selected={model.album === a.id} onclick={() => chooseAlbum(a.id)}>
              <span class="m-row-label">{a.title}</span><span class="count">{a.count}</span>
              {#if model.album === a.id}<Icon name="check" size={18} />{/if}
            </button>
          {/each}
        </div>
      </div>
    {/if}
  </div>
{/if}

<style>
  .gallery { flex: 1; padding: var(--sp-4) 0; }
  .tile.chosen img { transform: scale(0.92); }
  .empty-icon.tile { background: var(--accent-tint); color: var(--accent-text); }
  .pick { margin-top: var(--sp-2); }
  .cams { display: flex; gap: var(--sp-2); }
  .note { color: var(--text-2); font-size: var(--fs-xs); text-align: center; padding: var(--sp-1) 0; }

  /* Its own stack: the viewer and the albums float over the grid, not over the sheet. */
  .grid-panel { position: relative; isolation: isolate; flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .head { display: flex; align-items: center; gap: var(--sp-2); min-height: var(--control-h); flex-shrink: 0; }
  .album {
    display: inline-flex; align-items: center; gap: var(--sp-1); min-width: 0; max-width: 60%; padding: 0 var(--sp-2);
    min-height: var(--control-h-sm); border: 0; border-radius: var(--radius-pill); background: var(--m-seg); color: var(--text);
    font: inherit; font-size: var(--fs-sm); font-weight: var(--fw-semibold);
  }
  .album-title, .partial { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .partial { flex: 1; color: var(--text-2); font-size: var(--fs-xs); }
  .more { flex-shrink: 0; min-height: var(--control-h-sm); font-size: var(--fs-sm); }

  .scroll { flex: 1; min-height: 0; overflow-y: auto; overscroll-behavior: contain; touch-action: pan-y; border-radius: var(--radius-md); }
  .space { position: relative; }
  .cell { position: absolute; top: 0; left: 0; contain: strict; }
  .tile {
    width: 100%; height: 100%; padding: 0; border: 0; border-radius: var(--radius-xs); overflow: hidden;
    background: var(--surface-2); display: block; position: relative;
  }
  .tile img { width: 100%; height: 100%; object-fit: cover; display: block; transition: transform var(--dur-fast) var(--ease); }
  .tile:active img { transform: scale(0.97); }
  .dur {
    position: absolute; left: var(--sp-1); bottom: var(--sp-1); padding: 0 var(--sp-1); border-radius: var(--radius-xs);
    background: var(--media-scrim); color: #fff; font-size: var(--fs-2xs); font-weight: var(--fw-semibold); line-height: var(--lh-base);
  }
  .pick-circle {
    position: absolute; top: var(--sp-1); right: var(--sp-1); width: var(--media-check); height: var(--media-check); padding: 0;
    border-radius: var(--radius-pill); border: 2px solid #fff; background: var(--media-scrim-soft); color: #fff;
    font-size: var(--fs-xs); font-weight: var(--fw-bold); display: inline-flex; align-items: center; justify-content: center;
    box-shadow: var(--media-ring);
  }
  .pick-circle.on { background: var(--accent); border-color: var(--accent); }
  /* A larger place to hit than to see. */
  .pick-circle::before { content: ''; position: absolute; inset: calc(-1 * var(--sp-2)); }

  .camera {
    width: 100%; height: 100%; display: flex; flex-direction: column; border-radius: var(--radius-xs); overflow: hidden;
    background: var(--accent-tint); color: var(--accent-text);
  }
  .camera button {
    border: 0; background: transparent; color: inherit; font: inherit; font-size: var(--fs-xs); font-weight: var(--fw-semibold);
    display: flex; align-items: center; justify-content: center; gap: var(--sp-1); min-width: 0; padding: 0 var(--sp-1);
  }
  .camera span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cam-photo { flex: 1; flex-direction: column; }
  .cam-video { flex: 0 0 34%; border-top: 1px solid var(--accent-tint-border); }
  .camera button:active { background: var(--accent-bg); }

  .viewer {
    position: absolute; inset: 0; z-index: var(--z-panel); border-radius: var(--radius-md); overflow: hidden;
    background: var(--surface-2); display: flex; align-items: center; justify-content: center;
  }
  .viewer img { max-width: 100%; max-height: 100%; object-fit: contain; }
  .viewer .back { position: absolute; top: var(--sp-2); left: var(--sp-2); background: var(--glass-bg); }
  .viewer .pick-circle { top: var(--sp-3); right: var(--sp-3); width: var(--media-check-lg); height: var(--media-check-lg); }
  .play { position: absolute; color: #fff; filter: var(--media-icon-shadow); }
  .dur.big { left: var(--sp-3); bottom: var(--sp-3); font-size: var(--fs-xs); }

  .albums { position: absolute; inset: var(--control-h) 0 0; z-index: var(--z-panel); overflow-y: auto; }
  .none { padding: var(--sp-6) 0; }

  .add-bar { display: flex; align-items: center; gap: var(--sp-3); width: 100%; min-width: 0; }
  .orig { display: inline-flex; align-items: center; gap: var(--sp-2); min-width: 0; color: var(--text-2); font-size: var(--fs-sm); font-weight: var(--fw-semibold); }
  .orig span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .add { flex: 1 0 auto; width: auto; min-height: var(--control-h-lg); padding: 0 var(--sp-4); }
</style>
