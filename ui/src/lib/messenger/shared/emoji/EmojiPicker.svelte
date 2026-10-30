<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Emoji panel for any composer, and for a reaction in the menu of a message.
  A popover floats above the composer and closes on a tap outside; inline
  sits in the flow of whatever holds it, which also decides when it closes;
  dock stands in the place of the phone's keyboard (Composer), with «Стереть».
  Keys (design kit): Tab stops once at the group bar and once in the grid;
  in the bar ←/→, Home/End; in the grid ←/→ the next emoji, ↑/↓ the one above
  or below, Home/End the first or last of its group, PageUp/PageDown the
  group before or after; Enter picks.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import { EMOJI, loadRecent, pushRecent, type EmojiGroup } from './data';
  import { usageStore } from './usageStore.svelte';

  interface Props {
    onpick: (emoji: string) => void;
    onclose?: () => void;
    /** The dock's «Стереть»: the symbol before the caret. */
    onerase?: () => void;
    variant?: 'popover' | 'inline' | 'dock';
  }
  let { onpick, onclose, onerase, variant = 'popover' }: Props = $props();
  const inline = $derived(variant === 'inline');
  const docked = $derived(variant === 'dock');

  /** Recents of this device: shown only until the runtime names popular ones. */
  let local = $state<string[]>([]);
  const popular = $derived(usageStore.popular.length > 0);
  const recent = $derived(popular ? usageStore.popular : local);
  let active = $state<EmojiGroup['id'] | 'recent'>('smileys');
  let root = $state<HTMLDivElement | null>(null);
  let body = $state<HTMLDivElement | null>(null);

  onMount(() => {
    local = loadRecent();
    if (recent.length) active = 'recent';
    if (variant !== 'popover') return;
    // A tap outside closes the panel; taps inside never steal the focus.
    const outside = (e: PointerEvent) => {
      const target = e.target as Node;
      if (root && !root.contains(target) && !(target as HTMLElement).closest?.('[data-emoji-toggle]')) onclose?.();
    };
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') onclose?.(); };
    document.addEventListener('pointerdown', outside, true);
    window.addEventListener('keydown', key);
    return () => {
      document.removeEventListener('pointerdown', outside, true);
      window.removeEventListener('keydown', key);
    };
  });

  function pick(e: string) {
    local = pushRecent(e);
    onpick(e);
  }

  function go(id: EmojiGroup['id'] | 'recent') {
    active = id;
    body?.querySelector(`[data-group="${id}"]`)?.scrollIntoView({ block: 'start' });
  }

  const keep = (e: Event) => e.preventDefault();

  const tabButtons = () => [...(root?.querySelectorAll<HTMLButtonElement>('.tabs .tab') ?? [])];
  const items = () => [...(body?.querySelectorAll<HTMLButtonElement>('.grid button') ?? [])];

  /** Roving tabindex: one item of a list is the stop of Tab. */
  function rove(list: HTMLButtonElement[], n: number) {
    list.forEach((b, i) => (b.tabIndex = i === n ? 0 : -1));
    list[n].focus();
    list[n].scrollIntoView({ block: 'nearest' });
  }

  /** The grid keeps one stop of Tab even when its item went (a recent emoji moved). */
  function ensureStop() {
    const list = items();
    if (list.length && !list.some((b) => b.tabIndex === 0)) list[0].tabIndex = 0;
  }
  $effect(() => {
    void recent;
    queueMicrotask(ensureStop);
  });

  function tabsKey(e: KeyboardEvent) {
    const list = tabButtons();
    const i = list.indexOf(e.target as HTMLButtonElement);
    if (i < 0) return;
    const n = e.key === 'ArrowRight' ? (i + 1) % list.length
      : e.key === 'ArrowLeft' ? (i - 1 + list.length) % list.length
      : e.key === 'Home' ? 0 : e.key === 'End' ? list.length - 1 : -1;
    if (n < 0) return;
    e.preventDefault();
    list.forEach((b, k) => (b.tabIndex = k === n ? 0 : -1));
    list[n].focus();
  }

  /** The nearest item of the next (dir 1) or previous (-1) row, by its column. */
  function vertical(list: HTMLButtonElement[], i: number, dir: 1 | -1): number {
    const top = list[i].offsetTop, left = list[i].offsetLeft;
    let row: number | null = null, best = -1, bestDx = Infinity;
    for (let k = i + dir; k >= 0 && k < list.length; k += dir) {
      const t = list[k].offsetTop;
      if (t === top) continue;
      if (row === null) row = t;
      if (t !== row) break;
      const dx = Math.abs(list[k].offsetLeft - left);
      if (dx < bestDx) { bestDx = dx; best = k; }
    }
    return best;
  }

  function gridKey(e: KeyboardEvent) {
    const list = items();
    const it = e.target as HTMLButtonElement;
    const i = list.indexOf(it);
    if (i < 0) return;
    const grid = (b: HTMLElement) => b.parentElement;
    const inGrid = (g: HTMLElement | null) => list.filter((b) => grid(b) === g);
    let n = -1;
    if (e.key === 'ArrowRight') n = Math.min(list.length - 1, i + 1);
    else if (e.key === 'ArrowLeft') n = Math.max(0, i - 1);
    else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') n = vertical(list, i, e.key === 'ArrowDown' ? 1 : -1);
    else if (e.key === 'Home' || e.key === 'End') {
      const g = inGrid(grid(it));
      n = list.indexOf(e.key === 'Home' ? g[0] : g[g.length - 1]);
    } else if (e.key === 'PageDown' || e.key === 'PageUp') {
      const grids = [...(body?.querySelectorAll<HTMLElement>('.grid') ?? [])];
      const gi = grids.indexOf(grid(it)!) + (e.key === 'PageDown' ? 1 : -1);
      if (gi >= 0 && gi < grids.length) n = list.indexOf(inGrid(grids[gi])[0]);
    } else return;
    e.preventDefault();
    if (n < 0 || n === i) return;
    rove(list, n);
    const id = list[n].closest<HTMLElement>('[data-grid]')?.dataset.grid;
    if (id) active = id as EmojiGroup['id'] | 'recent';
  }
</script>

<div class="picker msg-font" class:inline class:docked bind:this={root} role="dialog" aria-label={$t('msg_emoji_title')}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="tabs" role="toolbar" tabindex="-1" onpointerdown={keep} onkeydown={tabsKey}>
    {#if recent.length}
      <button class="tab" class:active={active === 'recent'} tabindex={active === 'recent' ? 0 : -1} onclick={() => go('recent')} title={popular ? $t('msg_reactions_popular') : $t('msg_emoji_recent')}>🕘</button>
    {/if}
    {#each EMOJI as g (g.id)}
      <button class="tab" class:active={active === g.id} tabindex={active === g.id ? 0 : -1} onclick={() => go(g.id)} title={$t(`msg_emoji_${g.id}` as 'msg_emoji_smileys')}>{g.icon}</button>
    {/each}
    {#if docked && onerase}
      <button class="erase" onclick={onerase} aria-label={$t('msg_emoji_erase')} title={$t('msg_emoji_erase')}>
        <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 6a1 1 0 0 1 1 1v10a1 1 0 0 1 -1 1h-11l-5 -5a1.5 1.5 0 0 1 0 -2l5 -5z"/><path d="M12 10l4 4m0 -4l-4 4"/></svg>
      </button>
    {/if}
  </div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="body" bind:this={body} onpointerdown={keep} onkeydown={gridKey}>
    {#if recent.length}
      <div class="label" data-group="recent">{popular ? $t('msg_reactions_popular') : $t('msg_emoji_recent')}</div>
      <div class="grid" data-grid="recent">{#each recent as e (e)}<button tabindex="-1" onclick={() => pick(e)}>{e}</button>{/each}</div>
    {/if}
    {#each EMOJI as g (g.id)}
      <div class="label" data-group={g.id}>{$t(`msg_emoji_${g.id}` as 'msg_emoji_smileys')}</div>
      <div class="grid" data-grid={g.id}>{#each g.list as e (e)}<button tabindex="-1" onclick={() => pick(e)}>{e}</button>{/each}</div>
    {/each}
  </div>
</div>

<style>
  .picker {
    position: absolute; left: var(--sp-3); bottom: calc(100% + 6px); z-index: var(--z-popover, 50);
    width: min(360px, calc(100% - 2 * var(--sp-3))); height: 300px; display: flex; flex-direction: column;
    background: var(--glass-bg); -webkit-backdrop-filter: var(--glass-filter); backdrop-filter: var(--glass-filter);
    border: 1px solid var(--glass-border); border-radius: var(--radius-md); box-shadow: var(--shadow-lg), var(--glass-highlight);
    overflow: hidden;
  }
  .tabs { display: flex; gap: 2px; padding: 4px; border-bottom: 1px solid var(--border); overflow-x: auto; scrollbar-width: none; flex-shrink: 0; }
  .tabs button { flex: 1 0 auto; display: grid; place-items: center; min-width: 36px; height: 36px; border: none; background: none; font-size: var(--fs-lg); line-height: 1; padding: 0 var(--sp-1); border-radius: var(--radius); cursor: pointer; filter: grayscale(0.4); opacity: 0.75; }
  .tabs button.active { background: var(--accent-bg); filter: none; opacity: 1; }
  .body { flex: 1; min-height: 0; overflow-y: auto; padding: var(--sp-1) var(--sp-3) var(--sp-2); }
  .label { position: sticky; top: 0; background: var(--glass-bg); -webkit-backdrop-filter: var(--glass-filter); backdrop-filter: var(--glass-filter); font-size: var(--fs-xs); font-weight: var(--fw-bold); color: var(--text-2); padding: var(--sp-2) var(--sp-1) var(--sp-1); }
  .inline .label, .docked .label { background: var(--surface); -webkit-backdrop-filter: none; backdrop-filter: none; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(36px, 1fr)); }
  .grid button { border: none; background: none; font-size: 22px; line-height: 1; padding: 7px 0; border-radius: var(--radius); cursor: pointer; font-family: inherit; }
  .grid button:hover, .grid button:focus { background: var(--surface-3); outline: none; }
  .tabs button:focus-visible, .grid button:focus-visible { box-shadow: inset 0 0 0 1.5px var(--accent-border); outline: none; }
  /* In the flow of a menu or a sheet: as wide as it, no shadow of its own. */
  .picker.inline { position: static; width: 100%; height: 260px; box-shadow: none; background: var(--surface); -webkit-backdrop-filter: none; backdrop-filter: none; }
  /* In the place of the keyboard: the whole width and height of its slot, the opaque surface of a keyboard. */
  .picker.docked { position: static; width: 100%; height: 100%; border: none; border-top: 1px solid var(--border); border-radius: 0; box-shadow: none; background: var(--surface); -webkit-backdrop-filter: none; backdrop-filter: none; }
  .tabs .erase { margin-left: auto; position: sticky; right: 0; filter: none; opacity: 1; color: var(--text-soft); background: var(--surface); display: inline-flex; align-items: center; }
  .tabs .erase:active { color: var(--text); background: var(--surface-hover); }
  @media (pointer: coarse) {
    .picker:not(.inline):not(.docked) { left: 0; right: 0; width: 100%; border-radius: var(--radius-md) var(--radius-md) 0 0; border-inline: none; bottom: 100%; height: 280px; }
    .grid { grid-template-columns: repeat(auto-fill, minmax(42px, 1fr)); }
    .grid button { font-size: 26px; padding: 8px 0; }
  }
</style>
