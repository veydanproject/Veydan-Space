<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The part of a picked picture to keep as the avatar: a square viewport
  with a round mask over the picture, which always covers it. Drag to move
  (one finger or the mouse), zoom with the wheel, a pinch, the slider or
  the buttons; the arrow keys move and + / − zoom. `rect` is the square as
  fractions 0..1 of the picture, what `avatar_set` takes.

  `src` is the `data:` preview the runtime made; `width` and `height`
  give its proportions (of the picture itself, upright).
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import type { CropRect } from '../api';
  import { safeDataImage } from './avatars.svelte';
  import { clampCrop, initialCrop, layoutOf, maxZoom, panBy, rectOf, zoomAt, type Crop } from './profileEdit';

  interface Props {
    src: string;
    width: number;
    height: number;
    disabled?: boolean;
    /** Out: the square to keep. */
    rect?: CropRect;
  }
  let { src, width, height, disabled = false, rect = $bindable() }: Props = $props();

  let crop = $state<Crop>(initialCrop());
  let view = $state(0);
  let box = $state<HTMLElement | null>(null);

  const image = $derived(safeDataImage(src));
  const zmax = $derived(maxZoom(width, height));
  const layout = $derived(view > 0 ? layoutOf(crop, width, height, view) : null);

  // A new picture starts whole and centered.
  $effect(() => {
    void src; void width; void height;
    crop = initialCrop();
  });

  $effect(() => {
    rect = rectOf(crop, width, height);
  });

  // ── Pointers: one drags, two pinch ─────────────────────────────────
  const pointers = new Map<number, { x: number; y: number }>();
  let pinch: { dist: number; mx: number; my: number } | null = null;

  function local(x: number, y: number): { x: number; y: number } {
    const r = box?.getBoundingClientRect();
    return r ? { x: x - r.left, y: y - r.top } : { x, y };
  }

  function pinchOf(): { dist: number; mx: number; my: number } | null {
    if (pointers.size < 2) return null;
    const [a, b] = [...pointers.values()];
    const m = local((a.x + b.x) / 2, (a.y + b.y) / 2);
    return { dist: Math.hypot(a.x - b.x, a.y - b.y), mx: m.x, my: m.y };
  }

  function onDown(e: PointerEvent) {
    if (disabled || (e.pointerType === 'mouse' && e.button !== 0)) return;
    e.preventDefault();
    box?.focus({ preventScroll: true });
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    pinch = pinchOf();
  }

  function onMove(e: PointerEvent) {
    const prev = pointers.get(e.pointerId);
    if (!prev || disabled) return;
    const next = { x: e.clientX, y: e.clientY };
    pointers.set(e.pointerId, next);
    if (pointers.size === 1) {
      crop = panBy(crop, next.x - prev.x, next.y - prev.y, width, height, view);
      return;
    }
    const now = pinchOf();
    if (!now || !pinch) { pinch = now; return; }
    let c = crop;
    if (pinch.dist > 0 && now.dist > 0) c = zoomAt(c, c.z * (now.dist / pinch.dist), now.mx, now.my, width, height, view);
    c = panBy(c, now.mx - pinch.mx, now.my - pinch.my, width, height, view);
    crop = c;
    pinch = now;
  }

  function onUp(e: PointerEvent) {
    pointers.delete(e.pointerId);
    pinch = pinchOf();
  }

  function onWheel(e: WheelEvent) {
    if (disabled) return;
    e.preventDefault();
    const lines = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? view : 1;
    const p = local(e.clientX, e.clientY);
    crop = zoomAt(crop, crop.z * Math.exp(-e.deltaY * lines * 0.0015), p.x, p.y, width, height, view);
  }

  // ── Keys, slider, buttons ──────────────────────────────────────────
  const ZOOM_STEP = 1.15;

  function zoomBy(f: number) {
    crop = zoomAt(crop, crop.z * f, view / 2, view / 2, width, height, view);
  }

  function onKey(e: KeyboardEvent) {
    if (disabled) return;
    const step = e.shiftKey ? 40 : 10;
    // The picture moves the way the arrow points, as under a finger.
    const moves: Record<string, [number, number]> = {
      ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step],
    };
    const m = moves[e.key];
    if (m) { crop = panBy(crop, m[0], m[1], width, height, view); e.preventDefault(); return; }
    if (e.key === '+' || e.key === '=') { zoomBy(ZOOM_STEP); e.preventDefault(); return; }
    if (e.key === '-' || e.key === '_' || e.key === '−') { zoomBy(1 / ZOOM_STEP); e.preventDefault(); return; }
    if (e.key === '0') { crop = initialCrop(); e.preventDefault(); }
  }

  function onSlider(e: Event) {
    const z = Number((e.currentTarget as HTMLInputElement).value);
    crop = zoomAt(crop, z, view / 2, view / 2, width, height, view);
  }

  const atStart = $derived.by(() => {
    const k = clampCrop(crop, width, height);
    const s = clampCrop(initialCrop(), width, height);
    return Math.abs(k.z - s.z) < 1e-6 && Math.abs(k.cx - s.cx) < 1e-6 && Math.abs(k.cy - s.cy) < 1e-6;
  });
</script>

<!-- The viewport is a surface of its own: keys and pointers move and zoom the picture (the hint says how). -->
<div class="cropper" class:disabled>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div
    class="viewport"
    bind:this={box}
    bind:clientWidth={view}
    role="application"
    aria-label={$t('msg_profile_crop_area')}
    aria-roledescription={$t('msg_profile_crop_title')}
    tabindex={disabled ? -1 : 0}
    onpointerdown={onDown}
    onpointermove={onMove}
    onpointerup={onUp}
    onpointercancel={onUp}
    onlostpointercapture={onUp}
    onwheel={onWheel}
    onkeydown={onKey}
  >
    {#if image && layout}
      <img
        src={image}
        alt=""
        draggable="false"
        style:width="{layout.width}px"
        style:height="{layout.height}px"
        style:transform="translate({layout.left}px, {layout.top}px)"
      />
    {/if}
    <div class="mask" aria-hidden="true"></div>
  </div>

  <div class="zoom">
    <button type="button" class="icon-btn" {disabled} onclick={() => zoomBy(1 / ZOOM_STEP)} title={$t('msg_profile_crop_zoom_out')} aria-label={$t('msg_profile_crop_zoom_out')}>
      <Icon name="minus" size={16} />
    </button>
    <input
      type="range"
      min="1"
      max={zmax}
      step="0.01"
      value={crop.z}
      oninput={onSlider}
      disabled={disabled || zmax <= 1}
      aria-label={$t('msg_profile_crop_zoom')}
    />
    <button type="button" class="icon-btn" {disabled} onclick={() => zoomBy(ZOOM_STEP)} title={$t('msg_profile_crop_zoom_in')} aria-label={$t('msg_profile_crop_zoom_in')}>
      <Icon name="plus" size={16} />
    </button>
    <button type="button" class="icon-btn" disabled={disabled || atStart} onclick={() => (crop = initialCrop())} title={$t('msg_profile_crop_reset')} aria-label={$t('msg_profile_crop_reset')}>
      <Icon name="rotate-ccw" size={15} />
    </button>
  </div>
  <p class="hint">{$t('msg_profile_crop_hint')}</p>
</div>

<style>
  .cropper { display: flex; flex-direction: column; align-items: center; gap: var(--sp-3); width: 100%; }
  .viewport {
    position: relative; width: min(100%, 320px); aspect-ratio: 1; overflow: hidden;
    border-radius: var(--radius-md); background: var(--surface-3);
    cursor: grab; touch-action: none; user-select: none; -webkit-user-select: none;
    outline: none;
  }
  .viewport:active { cursor: grabbing; }
  .viewport:focus-visible { box-shadow: 0 0 0 2px var(--accent); }
  .disabled .viewport { cursor: default; opacity: 0.7; }
  img {
    position: absolute; left: 0; top: 0; max-width: none; transform-origin: 0 0;
    pointer-events: none; user-select: none; -webkit-user-drag: none;
  }
  /* The round avatar inside the square; the corners dimmed. */
  .mask {
    position: absolute; inset: 0; border-radius: 50%; pointer-events: none;
    box-shadow: 0 0 0 999px color-mix(in srgb, var(--bg) 62%, transparent), inset 0 0 0 1px color-mix(in srgb, var(--text) 45%, transparent);
  }
  .zoom { display: flex; align-items: center; gap: var(--sp-2); width: min(100%, 320px); }
  .zoom input[type='range'] { flex: 1; min-width: 0; padding: 0; border: none; background: none; box-shadow: none; accent-color: var(--accent); }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); text-align: center; max-width: 320px; }
  @media (pointer: coarse) {
    .zoom .icon-btn { width: 40px; height: 40px; }
  }
</style>
