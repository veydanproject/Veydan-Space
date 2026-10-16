<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Mount once per screen. Shows what `viewer.open` was given and pages through its album.
  A picture zooms: two fingers or a double tap on the phone; the buttons, the
  level menu, the wheel, a double click and the keys + − 0 on the desktop.
  Zoomed, a finger or the mouse moves it.
-->
<script lang="ts">
  import { page } from '$app/state';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { isTauriHost, messengerApi } from '../api';
  import { saveFilters } from '../shared/format';
  import { viewer } from './viewer.svelte';
  import { anchored, FIT, percent, stepped, ZOOM_MAX, ZOOM_STEPS, type View } from './zoom';

  /** How far a finger goes sideways before it pages. */
  const SWIPE_PX = 48;
  /** Two taps this close in time and place are a double tap. */
  const DOUBLE_TAP_MS = 300;
  const DOUBLE_TAP_PX = 30;
  /** The level a double tap or a double click zooms to. */
  const DOUBLE_SCALE = 2.5;

  let stage = $state<HTMLDivElement | null>(null);
  let img = $state<HTMLImageElement | null>(null);
  let view = $state<View>(FIT);
  /** A button, a double tap or a level from the menu glides; fingers and the wheel do not. */
  let smooth = $state(false);
  let levels = $state(false);
  /** How far the picture follows a finger that pages. */
  let drag = $state(0);

  const zoomed = $derived(view.scale > 1.001);
  const many = $derived(viewer.items.length > 1);
  const hasPrev = $derived(viewer.index > 0);
  const hasNext = $derived(viewer.index >= 0 && viewer.index < viewer.items.length - 1);
  const isImage = $derived(viewer.item?.kind === 'image');

  $effect(() => { if (viewer.item) { view = FIT; drag = 0; levels = false; } });
  // A back (the gesture, the button) left the viewer's step of history.
  $effect(() => { void page.state.mediaViewer; viewer.left(); });
  // The screen is left: what it showed does not wait for its return.
  $effect(() => () => viewer.close());
  // The wheel zooms instead of scrolling the page: a listener that may say no to it.
  $effect(() => {
    const el = stage;
    if (!el) return;
    el.addEventListener('wheel', wheel, { passive: false });
    return () => el.removeEventListener('wheel', wheel);
  });

  /** A point of the window, from the centre of the stage. */
  function local(x: number, y: number) {
    const r = stage?.getBoundingClientRect();
    return r ? { x: x - r.left - r.width / 2, y: y - r.top - r.height / 2 } : { x: 0, y: 0 };
  }
  const sizes = () => ({
    pic: { w: img?.offsetWidth ?? 0, h: img?.offsetHeight ?? 0 },
    screen: { w: stage?.clientWidth ?? 0, h: stage?.clientHeight ?? 0 },
  });

  /** Zooms to `scale` keeping the spot under `at` (the centre when none) where it is. */
  function zoomTo(scale: number, at = { x: 0, y: 0 }, glide = true) {
    if (!isImage) return;
    const { pic, screen } = sizes();
    smooth = glide;
    view = scale <= 1 ? FIT : anchored(view, scale, at, at, pic, screen);
  }
  function zoomStep(dir: 1 | -1) { zoomTo(stepped(view.scale, dir)); }
  /** A double tap or a double click: in to that spot, or back to fitting. */
  function toggleAt(x: number, y: number) { zoomTo(zoomed ? 1 : DOUBLE_SCALE, local(x, y)); }

  function wheel(e: WheelEvent) {
    if (!isImage) return;
    e.preventDefault();
    zoomTo(view.scale * Math.exp(-e.deltaY * 0.0015), local(e.clientX, e.clientY), false);
  }

  function key(e: KeyboardEvent) {
    if (!viewer.item) return;
    if (e.key === 'Escape') { e.stopPropagation(); if (levels) levels = false; else viewer.dismiss(); }
    else if (e.key === 'ArrowLeft' && hasPrev) { e.preventDefault(); viewer.step(-1); }
    else if (e.key === 'ArrowRight' && hasNext) { e.preventDefault(); viewer.step(1); }
    else if (e.key === '+' || e.key === '=') { e.preventDefault(); zoomStep(1); }
    else if (e.key === '-') { e.preventDefault(); zoomStep(-1); }
    else if (e.key === '0') { e.preventDefault(); zoomTo(1); }
  }

  // What the fingers (or the mouse) on the picture do. One finger: pages when
  // the picture fits, moves it when zoomed. Two: zoom about the spot between them.
  type Gesture =
    | { kind: 'swipe'; id: number; x: number; y: number }
    | { kind: 'pan'; id: number; x: number; y: number; from: View }
    | { kind: 'pinch'; d: number; at: { x: number; y: number }; from: View };
  const points = new Map<number, { x: number; y: number }>();
  let gesture: Gesture | null = null;
  /** The pointer moved: the click its lifting makes neither zooms nor closes. */
  let moved = false;
  let lastTap: { at: number; x: number; y: number } | null = null;
  /** The press began on the picture: its click (made on the stage, which holds the pointer) does not close the viewer. */
  let pressed = false;

  function begin(id: number, p: { x: number; y: number }, type: string) {
    if (zoomed) gesture = { kind: 'pan', id, x: p.x, y: p.y, from: view };
    else if (many && type !== 'mouse') gesture = { kind: 'swipe', id, x: p.x, y: p.y };
    else gesture = null;
  }
  function pinchStart() {
    const [a, b] = [...points.values()];
    drag = 0;
    gesture = { kind: 'pinch', d: Math.hypot(a.x - b.x, a.y - b.y) || 1, at: local((a.x + b.x) / 2, (a.y + b.y) / 2), from: view };
  }

  function down(e: PointerEvent) {
    pressed = false;
    if (!isImage || !(e.target instanceof HTMLImageElement) || (e.pointerType === 'mouse' && e.button !== 0)) return;
    if (points.size === 0) moved = false;
    pressed = true;
    points.set(e.pointerId, { x: e.clientX, y: e.clientY });
    stage?.setPointerCapture?.(e.pointerId);
    smooth = false;
    if (points.size === 2) pinchStart();
    else if (points.size === 1) begin(e.pointerId, { x: e.clientX, y: e.clientY }, e.pointerType);
  }

  function move(e: PointerEvent) {
    if (!points.has(e.pointerId)) return;
    points.set(e.pointerId, { x: e.clientX, y: e.clientY });
    const g = gesture;
    if (!g) return;
    if (g.kind === 'pinch') {
      if (points.size < 2) return;
      const [a, b] = [...points.values()];
      const { pic, screen } = sizes();
      const scale = g.from.scale * Math.hypot(a.x - b.x, a.y - b.y) / g.d;
      view = anchored(g.from, scale, g.at, local((a.x + b.x) / 2, (a.y + b.y) / 2), pic, screen);
      moved = true;
      return;
    }
    if (g.id !== e.pointerId) return;
    const dx = e.clientX - g.x;
    const dy = e.clientY - g.y;
    if (Math.abs(dx) + Math.abs(dy) > 6) moved = true;
    if (g.kind === 'pan') {
      const { pic, screen } = sizes();
      view = anchored(g.from, g.from.scale, { x: 0, y: 0 }, { x: dx, y: dy }, pic, screen);
    } else {
      // Past an end the picture only gives a little.
      drag = (dx > 0 && !hasPrev) || (dx < 0 && !hasNext) ? dx / 4 : dx;
    }
  }

  function up(e: PointerEvent) {
    const p = points.get(e.pointerId);
    if (!p) return;
    points.delete(e.pointerId);
    const g = gesture;
    if (g?.kind === 'pinch') {
      // The finger left on the picture goes on moving it.
      const [rest] = [...points.entries()];
      if (rest) begin(rest[0], rest[1], e.pointerType);
      else gesture = null;
      return;
    }
    gesture = null;
    if (g?.kind === 'swipe' && g.id === e.pointerId) {
      const dx = e.clientX - g.x;
      const dy = e.clientY - g.y;
      drag = 0;
      if (Math.abs(dx) >= SWIPE_PX && Math.abs(dx) >= Math.abs(dy) * 1.5) { moved = true; viewer.step(dx < 0 ? 1 : -1); return; }
    }
    if (!moved) {
      const now = Date.now();
      if (lastTap && now - lastTap.at < DOUBLE_TAP_MS && Math.hypot(p.x - lastTap.x, p.y - lastTap.y) < DOUBLE_TAP_PX) {
        lastTap = null;
        toggleAt(p.x, p.y);
      } else lastTap = { at: now, x: p.x, y: p.y };
    }
  }

  function lost(e: PointerEvent) {
    points.delete(e.pointerId);
    gesture = null;
    drag = 0;
  }

  function swallow(e: MouseEvent) {
    if (!moved && !pressed) return;
    moved = false;
    pressed = false;
    e.stopPropagation();
  }

  function backdrop() {
    if (levels) levels = false;
    else viewer.dismiss();
  }

  async function saveAs() {
    const it = viewer.item;
    if (!it || !isTauriHost) return;
    const { save } = await import('@tauri-apps/plugin-dialog');
    const dest = await save({ defaultPath: it.name, filters: saveFilters(it.name, $t('msg_media_all_files')) });
    if (dest) await messengerApi.media.saveAs(it.messageId, dest).catch(() => {});
  }
</script>

<svelte:window onkeydown={key} />

{#if viewer.item}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="viewer" role="dialog" aria-modal="true" aria-label={viewer.item.name} tabindex="-1" onclick={backdrop}>
    <header role="toolbar" tabindex="-1"
      onclick={(e) => { e.stopPropagation(); if (!(e.target instanceof Element && e.target.closest('.zoom'))) levels = false; }}>
      <span class="name">{viewer.item.name}</span>
      {#if many}<span class="count">{viewer.index + 1} / {viewer.items.length}</span>{/if}
      {#if isImage}
        <div class="zoom" class:zoomed>
          <button class="step" onclick={() => zoomStep(-1)} disabled={!zoomed} aria-label={$t('msg_media_zoom_out')} title={$t('msg_media_zoom_out')}>
            <Icon name="minus" size={18} />
          </button>
          <button class="level" onclick={() => (levels = !levels)} aria-haspopup="menu" aria-expanded={levels} title={$t('msg_media_zoom')}>
            {percent(view.scale)}
          </button>
          <button class="step" onclick={() => zoomStep(1)} disabled={view.scale >= ZOOM_MAX} aria-label={$t('msg_media_zoom_in')} title={$t('msg_media_zoom_in')}>
            <Icon name="plus" size={18} />
          </button>
          {#if levels}
            <div class="levels" role="menu">
              {#each ZOOM_STEPS as s (s)}
                <button role="menuitem" class:on={Math.abs(view.scale - s) < 0.01} onclick={() => { levels = false; zoomTo(s); }}>{percent(s)}</button>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
      <button onclick={saveAs} aria-label={$t('msg_media_save_as')} title={$t('msg_media_save_as')}><Icon name="save" size={18} /></button>
      <button onclick={() => viewer.dismiss()} aria-label={$t('msg_back')} title={$t('msg_back')}><Icon name="x" size={20} /></button>
    </header>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="stage" bind:this={stage} onclickcapture={swallow}
      onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={lost}>
      {#key viewer.item.messageId}
        {#if viewer.item.kind === 'image'}
          <img bind:this={img} src={viewer.item.src} alt={viewer.item.name} class:zoomed class:smooth draggable="false"
            style="transform: translate({view.x + drag}px, {view.y}px) scale({view.scale})" />
        {:else}
          <!-- svelte-ignore a11y_media_has_caption -->
          <video src={viewer.item.src} controls autoplay playsinline onclick={(e) => e.stopPropagation()}></video>
        {/if}
      {/key}
    </div>
    {#if hasPrev}
      <button class="nav prev" onclick={(e) => { e.stopPropagation(); viewer.step(-1); }} aria-label={$t('msg_media_prev')} title={$t('msg_media_prev')}>
        <Icon name="chevron-left" size={28} />
      </button>
    {/if}
    {#if hasNext}
      <button class="nav next" onclick={(e) => { e.stopPropagation(); viewer.step(1); }} aria-label={$t('msg_media_next')} title={$t('msg_media_next')}>
        <Icon name="chevron-right" size={28} />
      </button>
    {/if}
  </div>
{/if}

<style>
  .viewer { position: fixed; inset: var(--overlay-inset, 0); border-radius: var(--overlay-radius, 0); overflow: hidden; z-index: var(--z-modal, 1000); background: rgba(6, 6, 10, 0.95); display: flex; flex-direction: column; }
  header {
    position: relative; z-index: 1; display: flex; align-items: center; gap: var(--sp-2); padding: calc(var(--sp-2) + var(--sat, 0px)) var(--sp-3) var(--sp-2);
    color: #fff; flex-shrink: 0;
  }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: var(--fs-sm); opacity: 0.85; }
  .count { flex-shrink: 0; font-size: var(--fs-sm); opacity: 0.85; font-variant-numeric: tabular-nums; }
  header button { width: 44px; height: 44px; border: none; border-radius: 50%; background: none; color: #fff; display: inline-flex; align-items: center; justify-content: center; cursor: pointer; }
  header button:hover:not(:disabled) { background: rgba(255, 255, 255, 0.12); }
  header button:disabled { opacity: 0.35; cursor: default; }
  .zoom { position: relative; display: flex; align-items: center; flex-shrink: 0; }
  .zoom .level { width: auto; min-width: 60px; padding: 0 var(--sp-2); border-radius: 22px; font-size: var(--fs-sm); font-variant-numeric: tabular-nums; }
  .levels {
    position: absolute; top: 100%; left: 50%; transform: translateX(-50%); margin-top: 4px; padding: 4px; display: flex; flex-direction: column;
    background: rgba(30, 30, 36, 0.98); border: 1px solid rgba(255, 255, 255, 0.12); border-radius: var(--radius-sm, 8px); box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
  }
  .levels button { width: 88px; height: 36px; border-radius: var(--radius-badge); font-size: var(--fs-sm); font-variant-numeric: tabular-nums; }
  .levels button.on { background: rgba(255, 255, 255, 0.16); }
  /* A phone zooms with two fingers: no buttons, the level only while zoomed. */
  @media (pointer: coarse) {
    .zoom .step { display: none; }
    .zoom:not(.zoomed) { display: none; }
  }
  /* Every finger on the stage belongs to the viewer: it pages, zooms and moves the picture. */
  .stage { position: relative; flex: 1; min-height: 0; display: flex; align-items: center; justify-content: center; overflow: hidden; touch-action: none; }
  img { max-width: 100%; max-height: 100%; object-fit: contain; cursor: zoom-in; user-select: none; -webkit-user-drag: none; transform-origin: center; }
  img.zoomed { cursor: grab; }
  img.zoomed:active { cursor: grabbing; }
  img.smooth { transition: transform 0.18s var(--ease, ease-out); }
  video { max-width: 100%; max-height: 100%; background: #000; }
  .nav {
    position: absolute; top: 50%; transform: translateY(-50%); width: 48px; height: 48px; border: none; border-radius: 50%;
    background: rgba(0, 0, 0, 0.45); color: #fff; display: inline-flex; align-items: center; justify-content: center; cursor: pointer;
  }
  .nav:hover { background: rgba(255, 255, 255, 0.18); }
  .nav.prev { left: var(--sp-3); }
  .nav.next { right: var(--sp-3); }
</style>
