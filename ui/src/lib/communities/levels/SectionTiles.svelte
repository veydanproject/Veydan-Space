<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The sections of a channel as tiles (communities-wire §14.7, the redesign
  of the channels): «Все» first, then each section in the owner's order —
  its mark, its title, the line under it, what is unread in it. The one
  shown is ringed in its colour; a locked one says which level opens it; a
  hidden one (shown to its members alone) says so and has a dashed edge.
  Under the button of the head on a desktop, a sheet on a phone. The owner
  and the admins see «Настроить».
-->
<script lang="ts">
  import { countKey, locale, t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';
  import { portal } from '$lib/core/portal';
  import { onPhone } from '$lib/messenger/seams/shared';
  import SectionBadge from './SectionBadge.svelte';
  import { lookOf } from './look';
  import { scopePhotos } from './scopePhotos.svelte';
  import { EVERYONE, type SectionTile } from './sections';
  import type { ScopeView } from './types';

  interface Props {
    open: boolean;
    tiles: SectionTile[];
    communityId: string;
    /** The section shown; `null`: «Все». */
    current: string | null;
    /** What is unread in the whole channel, for «Все». */
    unread: number;
    /** The button the panel hangs under (a desktop). */
    anchor?: HTMLElement | null;
    onpick: (scopeId: string | null) => void;
    onlocked: (tile: Extract<SectionTile, { kind: 'locked' }>) => void;
    /** The owner's and the admins' way to the settings of the sections. */
    onsettings?: () => void;
  }
  let { open = $bindable(), tiles, communityId, current, unread, anchor = null, onpick, onlocked, onsettings }: Props = $props();

  const phone = onPhone();
  const label = (s: ScopeView) => (s.scope_id === EVERYONE && !s.title ? $t('chan_scope_everyone') : s.title);

  /**
   * Where the panel stands: under the button, its right edge on the button's,
   * never past the pane of the feed the button is in (12 from its edges); in
   * a narrow pane its whole width, the tiles in two columns.
   */
  const GAP = 12;
  const WIDTH = 600;
  let at = $state({ top: 0, left: 0, width: WIDTH, height: 0, cols: 3 });
  $effect(() => {
    if (!open || phone || !anchor) return;
    const place = () => {
      const r = anchor!.getBoundingClientRect();
      const box = (anchor!.closest('.window') ?? document.documentElement).getBoundingClientRect();
      const width = Math.round(Math.min(WIDTH, box.width - GAP * 2));
      const left = Math.round(Math.min(Math.max(box.left + GAP, r.right - width), box.right - GAP - width));
      const top = Math.round(r.bottom + 8);
      at = { top, left, width, height: Math.round(Math.min(box.bottom, window.innerHeight) - top - GAP), cols: width >= 520 ? 3 : 2 };
    };
    place();
    window.addEventListener('resize', place);
    return () => window.removeEventListener('resize', place);
  });

  // Opened from the keyboard, the panel takes the focus: the tile shown first.
  let panel = $state<HTMLDivElement | null>(null);
  $effect(() => {
    if (open && !phone && panel) (panel.querySelector('[aria-current="true"]') as HTMLElement | null)?.focus({ preventScroll: true });
  });

  function close() {
    open = false;
    if (!phone) anchor?.focus({ preventScroll: true });
  }

  function onkeydown(e: KeyboardEvent) {
    if (open && !phone && e.key === 'Escape') { e.stopPropagation(); close(); }
  }

  /** What the line under a tile says: why it is locked, its note, or that it is the one shown. */
  function sub(tile: SectionTile): string {
    if (tile.kind === 'all') return $t('chan_scope_all_note');
    if (tile.kind === 'locked') {
      return tile.levels.length
        ? $t(countKey('chan_scope_opens_by', tile.levels.length, $locale), { levels: tile.levels.map((l) => `«${l.name}»`).join(', ') })
        : $t('chan_scope_team_only');
    }
    return tile.scope.note || (tile.scope.scope_id === current ? $t('chan_scope_current') : '');
  }

  const isCurrent = (tile: SectionTile) => (tile.kind === 'all' ? current === null : tile.scope.scope_id === current);

  function pick(tile: SectionTile) {
    close();
    if (tile.kind === 'locked') onlocked(tile);
    else onpick(tile.kind === 'all' ? null : tile.scope.scope_id);
  }
</script>

<svelte:window onkeydown={onkeydown} />

{#snippet grid()}
  <div class="head">
    <span class="h-title">{$t('chan_sections')}</span>
    {#if onsettings}
      <button type="button" class="btn btn-ghost btn-sm settings" onclick={() => { close(); onsettings?.(); }}><Icon name="adjustments-horizontal" size={16} />{$t('chan_sections_settings')}</button>
    {/if}
  </div>
  <div class="grid">
    {#each tiles as tile (tile.kind === 'all' ? '*' : tile.scope.scope_id)}
      {@const scope = tile.kind === 'all' ? null : tile.scope}
      {@const look = scope ? lookOf(scope) : null}
      {@const photo = scope && look?.photo ? scopePhotos.src(communityId, scope.scope_id, look.photo) : null}
      {@const count = tile.kind === 'all' ? unread : tile.kind === 'open' ? (scope?.unread ?? 0) : 0}
      {@const name = scope ? label(scope) : $t('chan_scope_all')}
      {@const line = sub(tile)}
      <button type="button" class="tile" class:current={isCurrent(tile)} class:locked={tile.kind === 'locked'} class:hidden={!!scope?.hidden} class:photo={!!photo}
        aria-current={isCurrent(tile) ? "true" : undefined} style:--t-color={look?.color ?? 'var(--accent)'}
        aria-label={[name, line, tile.kind === 'locked' ? $t('chan_scope_locked') : '', scope?.hidden ? $t('chan_scope_hidden_mark') : ''].filter(Boolean).join(', ')}
        onclick={() => pick(tile)}>
        {#if photo}<img class="bg" src={photo} alt="" /><span class="shade"></span>{/if}
        <span class="top">
          <SectionBadge {scope} {communityId} size={34} noPhoto />
          <span class="marks">
            {#if tile.kind === 'locked'}<span class="lock"><Icon name="lock" size={14} /></span>
            {:else if scope?.hidden}<span class="hidden-mark"><Icon name="eye-off" size={14} />{$t('chan_scope_hidden_mark')}</span>
            {:else if count > 0}<span class="count-pill lg">{count > 99 ? '99+' : count}</span>{/if}
          </span>
        </span>
        <span class="words">
          <span class="name">{name}</span>
          {#if line}<span class="sub">{line}</span>{/if}
        </span>
      </button>
    {/each}
  </div>
  {#if tiles.some((x) => x.kind !== 'all' && x.scope.hidden)}<p class="hint"><Icon name="eye-off" size={14} /><span>{$t('chan_scope_hidden_hint')}</span></p>{/if}
{/snippet}

{#if phone}
  <BottomSheet {open} onclose={close} closable={false}>
    <div class="sheet">{@render grid()}</div>
  </BottomSheet>
{:else if open}
  <div class="catch" use:portal role="presentation" onclick={close}></div>
  <div class="panel" bind:this={panel} use:portal role="dialog" tabindex="-1" aria-label={$t('chan_sections')} style:top="{at.top}px" style:left="{at.left}px"
    style:width="{at.width}px" style:max-height="{at.height}px" style:--cols={at.cols}>
    {@render grid()}
  </div>
{/if}

<style>
  .catch { position: fixed; inset: var(--overlay-inset, 0); z-index: calc(var(--z-popover) - 1); }
  /* Under its button, never past the pane of the feed (its place is computed above). */
  .panel {
    position: fixed; z-index: var(--z-popover); overflow-y: auto; box-sizing: border-box;
    display: flex; flex-direction: column; gap: var(--sp-3); padding: var(--sp-4);
    border-radius: var(--radius-pane); background: var(--glass-bg); border: 1px solid var(--glass-border);
    box-shadow: var(--glass-highlight), var(--shadow-lg); -webkit-backdrop-filter: var(--glass-filter); backdrop-filter: var(--glass-filter);
  }
  .sheet { display: flex; flex-direction: column; gap: var(--sp-3); padding: 0 var(--sp-1) var(--sp-3); }
  .head { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); min-height: 32px; }
  .h-title { font-size: var(--fs-md); font-weight: var(--fw-bold); color: var(--text); }
  .sheet .h-title { font-size: var(--fs-lg); }
  .settings { gap: 6px; color: var(--accent-text); border-color: transparent; background: none; }
  .settings:hover { background: var(--accent-tint); }
  /* One scheme for every tile: its icon on its colour, its marks on the right; the name and its note under. */
  .grid { display: grid; grid-template-columns: repeat(var(--cols, 3), minmax(0, 1fr)); gap: var(--sp-2); }
  .sheet .grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .tile {
    position: relative; min-height: 112px; box-sizing: border-box; overflow: hidden; display: flex; flex-direction: column; justify-content: space-between; gap: 14px;
    padding: var(--sp-3); border-radius: var(--radius-lg); font: inherit; color: var(--text); text-align: left; cursor: pointer;
    border: 1px solid color-mix(in srgb, var(--t-color) 30%, var(--border)); background: color-mix(in srgb, var(--t-color) 10%, var(--surface-3));
    transition: border-color var(--dur-fast), background var(--dur-fast);
  }
  .sheet .tile { min-height: 116px; }
  .tile:hover { background: color-mix(in srgb, var(--t-color) 16%, var(--surface-3)); }
  .tile:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  /* The section shown: ringed in the accent's ink. */
  .tile.current { border-color: var(--accent-text); box-shadow: inset 0 0 0 1px var(--accent-text); }
  .tile.locked .top :global(.sec-mark), .tile.locked .words { opacity: 0.6; }
  .tile.hidden { border-style: dashed; border-color: color-mix(in srgb, var(--t-color) 55%, transparent); }
  /* Over its background the section keeps its one colour: the plate of the icon and a frame of 2px. */
  .tile.photo { color: #fff; border-width: 2px; border-color: var(--t-color); padding: calc(var(--sp-3) - 1px); }
  /* Its frame is already 2px: the accent's alone, no ring inside it. */
  .tile.photo.current { border-color: var(--accent-text); box-shadow: none; }
  .bg { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; }
  .shade { position: absolute; inset: 0; background: linear-gradient(180deg, rgba(0, 0, 0, 0.05) 30%, rgba(0, 0, 0, 0.62)); }
  .top { position: relative; align-self: stretch; display: flex; align-items: flex-start; justify-content: space-between; gap: var(--sp-2); }
  .marks { display: flex; align-items: center; gap: 6px; min-width: 0; margin-left: auto; }
  /* The count is the .count-pill of base.css, 22. */
  .lock { width: 22px; height: 22px; border-radius: var(--radius-pill); display: inline-flex; align-items: center; justify-content: center; background: var(--surface-2); color: var(--text-3); }
  .hidden-mark {
    display: inline-flex; align-items: center; gap: 4px; height: 22px; padding: 0 var(--sp-2); box-sizing: border-box; border-radius: var(--radius-pill);
    background: var(--danger-bg); color: var(--danger-text); font-size: var(--fs-xs); font-weight: var(--fw-semibold); white-space: nowrap;
  }
  .words { position: relative; align-self: stretch; display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .name { font-size: var(--fs-base); font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub {
    font-size: var(--fs-xs); line-height: 1.35; color: var(--text-2); overflow: hidden;
    display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2;
  }
  .photo .sub { color: rgba(255, 255, 255, 0.8); }
  .hint { display: flex; align-items: flex-start; gap: var(--sp-2); margin: 0; padding-top: var(--sp-3); border-top: 1px solid var(--divider); font-size: var(--fs-sm); color: var(--text-3); line-height: 1.45; }
  .hint :global(svg) { flex-shrink: 0; margin-top: 2px; }
  /* A tablet: three columns in the sheet. */
  @media (min-width: 600px) { .sheet .grid { grid-template-columns: repeat(3, minmax(0, 1fr)); } }
</style>
