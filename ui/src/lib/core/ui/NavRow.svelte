<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A row that leads somewhere: its icon, its title, a line of what is inside,
  what stands at its end (a count, a state, a control) and the chevron. The
  whole row is the link (`href`) or the button (`onclick`); without either,
  or `disabled`, it only tells.

  `variant` is where it stands:
  - `panel` (the default) — the contents of an info panel (a channel's, a
    group's): 56 (60 on touch), `--radius-md`, in a card of rows; the root is
    `.nav`, and `page` names it, so the focus comes back to it from its page.
  - `list` — a row of a list of the cloud: 64, a hairline under it, the tile
    in the accent by default, the end in place of the chevron.
  - `compact` — a line of a block (what a chat has shared): 46 (50 on touch),
    the icon without a tile, the count as a quiet number.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { isMobile } from '$lib/core/platform';
  import Tile, { type TileTone } from './Tile.svelte';
  import Chev from './Chev.svelte';

  interface Props {
    icon: string;
    title: string;
    variant?: 'panel' | 'list' | 'compact';
    /** The colour of the tile; `plain` in a panel, `accent` in a list. */
    tone?: TileTone;
    /** The title in the danger colour: a row that ends something. */
    danger?: boolean;
    /** The line under the title. */
    sub?: Snippet | string;
    /** What stands at the end, before the chevron. */
    end?: Snippet;
    /** A count at the end: a quiet `.count-pill`, a quiet number in `compact`. */
    count?: number | string;
    /** The chevron; by default, when the row leads somewhere. */
    chevron?: boolean;
    /** The title in one line, cut with «…»: room for a pill at the end of a narrow panel. */
    nowrap?: boolean;
    id?: string;
    /** The name of the page the row opens: the focus comes back to the row from it. */
    page?: string;
    /** The row leads somewhere: the whole of it is the link. */
    href?: string;
    /** The row does something: the whole of it is the button. */
    onclick?: () => void;
    disabled?: boolean;
  }
  let {
    icon, title, variant = 'panel', tone, danger = false, sub, end, count, chevron, nowrap = false,
    id, page, href, onclick, disabled = false,
  }: Props = $props();

  const link = $derived(!!href && !disabled);
  const button = $derived(!link && !!onclick && !disabled);
  const chev = $derived(chevron ?? !!(href || onclick));
  const tileTone = $derived(tone ?? (variant === 'list' ? 'accent' : 'plain'));
</script>

{#snippet body()}
  {#if variant === 'compact'}
    <span class="ico"><Icon name={icon} size={18} /></span>
  {:else}
    <Tile {icon} tone={tileTone} />
  {/if}
  <span class="text">
    <span class="title" class:nowrap>{title}</span>
    {#if sub}<span class="sub">{#if typeof sub === 'string'}{sub}{:else}{@render sub()}{/if}</span>{/if}
  </span>
  {#if count !== undefined}
    {#if variant === 'compact'}<span class="count">{count}</span>{:else}<b class="count-pill quiet">{count}</b>{/if}
  {/if}
  {#if end}
    {#if variant === 'list'}<span class="trailing">{@render end()}</span>{:else}{@render end()}{/if}
  {/if}
  {#if chev}<Chev />{/if}
{/snippet}

{#if link}
  <a class="navrow {variant} hit" class:nav={variant === 'panel'} class:m={isMobile} class:danger {id} data-page={page} {href}>{@render body()}</a>
{:else if button}
  <button type="button" class="navrow {variant} hit" class:nav={variant === 'panel'} class:m={isMobile} class:danger {id} data-page={page} {onclick}>{@render body()}</button>
{:else}
  <div class="navrow {variant}" class:nav={variant === 'panel'} class:m={isMobile} class:danger class:off={disabled} {id} data-page={page}>{@render body()}</div>
{/if}

<style>
  .navrow {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%;
    border: none; background: none; color: var(--text); font: inherit; text-align: left; text-decoration: none;
  }
  .hit { cursor: pointer; transition: background var(--dur-fast) var(--ease); }
  .hit:hover { background: var(--surface-row-hover); }
  .hit:focus-visible { outline: 2px solid var(--accent-border); outline-offset: -2px; }
  .off { opacity: 0.55; }
  .text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .title.nowrap { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  /* The contents of an info panel. */
  .panel { position: relative; min-height: 56px; padding: var(--sp-2) var(--sp-2) var(--sp-2) var(--sp-3); border-radius: var(--radius-md); }
  .panel .title { font-size: var(--fs-base); font-weight: var(--fw-bold); }
  .panel .sub { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: var(--fs-xs); line-height: 1.4; color: var(--text-2); }
  .panel .sub:has(:global(.stack)) { overflow: visible; }

  /* A list of the cloud. */
  .list { min-height: 64px; padding: var(--sp-3) var(--sp-5); border-bottom: 1px solid var(--divider); border-radius: 0; }
  .list:last-child { border-bottom: 0; }
  .list.m { padding: var(--sp-3) var(--sp-4); }
  .list .title { font-size: var(--fs-base); font-weight: var(--fw-semibold); line-height: 1.3; }
  .list.m .title { font-size: var(--fs-body); }
  .list.danger .title { color: var(--danger-text); }
  .list .sub { font-size: var(--fs-sm); color: var(--text-2); line-height: 1.4; }
  .trailing { flex-shrink: 0; display: inline-flex; align-items: center; gap: var(--sp-2); }

  /* A line of a block. */
  .compact { min-height: 46px; padding: 0 var(--sp-3); border-radius: 0; font-size: var(--fs-sm); }
  .compact .ico { display: inline-flex; color: var(--accent-text-2); }
  .compact .title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .compact .count { color: var(--text-3); font-variant-numeric: tabular-nums; }

  @media (pointer: coarse) {
    .panel { min-height: 60px; }
    .compact { min-height: 50px; }
  }
</style>
