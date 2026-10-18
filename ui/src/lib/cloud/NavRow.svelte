<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- A row of a list of the cloud: an icon, what it is and how it stands, and either the way further or a control of its own. -->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { isMobile } from '$lib/core/platform';

  let {
    icon,
    title,
    sub = '',
    href,
    onclick,
    tone,
    disabled = false,
    trailing,
  }: {
    icon: string;
    title: string;
    sub?: string;
    /** The row leads somewhere: the whole of it is the link. */
    href?: string;
    /** The row does something: the whole of it is the button. */
    onclick?: () => void;
    tone?: 'warn' | 'danger';
    disabled?: boolean;
    /** A control at the right, in place of the chevron. */
    trailing?: Snippet;
  } = $props();
</script>

{#snippet body()}
  <span class="tile {tone ?? ''}"><Icon name={icon} size={18} /></span>
  <span class="words">
    <span class="title">{title}</span>
    {#if sub}<span class="sub">{sub}</span>{/if}
  </span>
  {#if trailing}
    <span class="trailing">{@render trailing()}</span>
  {:else if href || onclick}
    <span class="chev"><Icon name="chevron-right" size={16} /></span>
  {/if}
{/snippet}

{#if href && !disabled}
  <a class="row {tone ?? ''}" class:m={isMobile} class:hit={true} {href}>{@render body()}</a>
{:else if onclick && !disabled}
  <button class="row {tone ?? ''}" class:m={isMobile} class:hit={true} type="button" {onclick}>{@render body()}</button>
{:else}
  <div class="row {tone ?? ''}" class:m={isMobile} class:off={disabled}>{@render body()}</div>
{/if}

<style>
  .row {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; min-height: 64px;
    padding: var(--sp-3) var(--sp-5); border: 0; border-bottom: 1px solid var(--divider); border-radius: 0;
    background: none; color: var(--text); font: inherit; text-align: left; text-decoration: none;
  }
  .row:last-child { border-bottom: 0; }
  .row.m { padding: var(--sp-3) var(--sp-4); }
  .hit { cursor: pointer; transition: background var(--dur-fast); }
  .hit:hover { background: var(--surface-row-hover); }
  .off { opacity: 0.55; }
  .tile {
    display: inline-flex; align-items: center; justify-content: center; width: 36px; height: 36px; flex-shrink: 0;
    border-radius: var(--radius); background: var(--accent-tint); color: var(--accent-text);
  }
  .tile.warn { background: var(--warn-bg); color: var(--warn-text); }
  .tile.danger { background: var(--danger-bg); color: var(--danger-text); }
  .words { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .title { font-size: var(--fs-base); font-weight: var(--fw-semibold); line-height: 1.3; }
  .m .title { font-size: var(--fs-body); }
  .danger .title { color: var(--danger-text); }
  .sub { font-size: var(--fs-sm); color: var(--text-2); line-height: 1.4; }
  .m .sub { font-size: var(--fs-sm); }
  .trailing { flex-shrink: 0; display: inline-flex; align-items: center; gap: var(--sp-2); }
  .chev { flex-shrink: 0; display: inline-flex; color: var(--text-3); }
</style>
