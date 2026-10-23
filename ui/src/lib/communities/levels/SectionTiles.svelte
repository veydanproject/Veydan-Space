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

  /** Where the panel stands: under the button, its right edge on the button's. */
  let at = $state({ top: 0, right: 0 });
  $effect(() => {
    if (!open || phone || !anchor) return;
    const place = () => {
      const r = anchor!.getBoundingClientRect();
      at = { top: Math.round(r.bottom + 8), right: Math.max(8, Math.round(window.innerWidth - r.right)) };
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
      <button type="button" class="settings" onclick={() => { close(); onsettings?.(); }}><Icon name="adjustments-horizontal" size={16} />{$t('chan_sections_settings')}</button>
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
          {#if photo}<span class="on-photo"><Icon name={look?.icon ?? 'folder'} size={18} /></span>
          {:else}<SectionBadge {scope} {communityId} size={36} />{/if}
          {#if tile.kind === 'locked'}<span class="lock"><Icon name="lock" size={16} /></span>
          {:else if scope?.hidden}<span class="hidden-mark"><Icon name="eye-off" size={13} />{$t('chan_scope_hidden_mark')}</span>
          {:else if count > 0}<span class="count">{count > 99 ? '99+' : count}</span>{/if}
        </span>
        <span class="words">
          <span class="name">{name}</span>
          {#if line}<span class="sub">{line}</span>{/if}
        </span>
      </button>
    {/each}
  </div>
  {#if tiles.some((x) => x.kind !== 'all' && x.scope.hidden)}<p class="hint">{$t('chan_scope_hidden_hint')}</p>{/if}
{/snippet}

{#if phone}
  <BottomSheet {open} onclose={close} closable={false}>
    <div class="sheet">{@render grid()}</div>
  </BottomSheet>
{:else if open}
  <div class="catch" use:portal role="presentation" onclick={close}></div>
  <div class="panel" bind:this={panel} use:portal role="dialog" tabindex="-1" aria-label={$t('chan_sections')} style:top="{at.top}px" style:right="{at.right}px">
    {@render grid()}
  </div>
{/if}

<style>
  .catch { position: fixed; inset: var(--overlay-inset, 0); z-index: 900; }
  .panel {
    position: fixed; z-index: 901; width: min(560px, calc(100vw - 32px)); max-height: calc(100vh - 120px); overflow-y: auto;
    display: flex; flex-direction: column; gap: var(--sp-3); padding: var(--sp-4); box-sizing: border-box;
    border-radius: var(--radius-lg, 20px); background: var(--glass-bg); border: 1px solid var(--glass-border);
    box-shadow: var(--glass-highlight), var(--shadow-lg); -webkit-backdrop-filter: var(--pane-filter); backdrop-filter: var(--pane-filter);
  }
  .sheet { display: flex; flex-direction: column; gap: var(--sp-3); padding: 0 var(--sp-1) var(--sp-3); }
  .head { display: flex; align-items: center; gap: var(--sp-2); }
  .h-title { flex: 1; font-size: var(--fs-md); font-weight: var(--fw-bold); color: var(--text); }
  .sheet .h-title { font-size: 18px; }
  .settings {
    display: inline-flex; align-items: center; gap: 6px; border: none; background: none; cursor: pointer; font: inherit;
    font-size: var(--fs-xs); font-weight: var(--fw-medium); color: var(--accent-text-2); padding: 6px 8px; border-radius: var(--radius-sm);
  }
  .settings:hover { background: var(--accent-tint); }
  .grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 10px; }
  .sheet .grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .tile {
    position: relative; height: 120px; box-sizing: border-box; overflow: hidden; display: flex; flex-direction: column; justify-content: space-between;
    padding: 12px; border-radius: 16px; font: inherit; color: var(--text); text-align: left; cursor: pointer;
    border: 1px solid color-mix(in srgb, var(--t-color) 28%, transparent); background: color-mix(in srgb, var(--t-color) 10%, var(--pane-strong));
    transition: border-color var(--dur-fast, 120ms), background var(--dur-fast, 120ms);
  }
  .sheet .tile { height: 112px; border-radius: 18px; }
  .tile:hover { background: color-mix(in srgb, var(--t-color) 16%, var(--pane-strong)); }
  .tile:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  /* The section shown: ringed in its colour. */
  .tile.current { border: 2px solid var(--t-color); padding: 11px; background: color-mix(in srgb, var(--t-color) 18%, var(--pane-strong)); }
  .tile.locked { background: var(--pane-strong); border-color: var(--pane-strong-border); }
  .tile.locked .top :global(.badge), .tile.locked .words { opacity: 0.7; }
  .tile.hidden { border-style: dashed; }
  .tile.photo { color: #fff; border-color: transparent; }
  .bg { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; }
  .shade { position: absolute; inset: 0; background: linear-gradient(180deg, rgba(0, 0, 0, 0.05) 30%, rgba(0, 0, 0, 0.62)); }
  .top { position: relative; display: flex; align-items: flex-start; justify-content: space-between; gap: 6px; }
  .on-photo { width: 36px; height: 36px; border-radius: 11px; display: inline-flex; align-items: center; justify-content: center; background: rgba(0, 0, 0, 0.35); color: #fff; }
  .count {
    min-width: 22px; height: 22px; padding: 0 6px; box-sizing: border-box; border-radius: 11px; display: inline-flex; align-items: center; justify-content: center;
    background: var(--accent); color: #fff; font-size: var(--fs-2xs); font-weight: var(--fw-semibold); font-variant-numeric: tabular-nums;
  }
  .lock { color: var(--text-2); display: inline-flex; padding-top: 2px; }
  .photo .lock { color: #fff; }
  .hidden-mark { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-2xs); font-weight: var(--fw-medium); color: var(--danger-text); }
  .photo .hidden-mark { color: #fff; }
  .words { position: relative; display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-semibold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { font-size: var(--fs-2xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tile.current .sub { color: color-mix(in srgb, var(--t-color) 55%, var(--text)); }
  .photo .sub { color: rgba(255, 255, 255, 0.86); }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.45; }
  @media (max-width: 520px) { .grid { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
</style>
