<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The «?» beside a term where a thing is made (a new level, a new section):
  the line that tells what the term means, opened under the row of the term
  (up to 440 wide, from its left) and closed by the same button. The row is the nearest
  positioned box around it (`position: relative`); the note points at the «?».
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';

  interface Props {
    /** The term, as the button's name. */
    term: string;
    open?: boolean;
    children: Snippet;
  }
  let { term, open = $bindable(false), children }: Props = $props();

  const id = $props.id();
  let btn: HTMLButtonElement | undefined = $state();
  let ax = $state(0);

  function toggle() {
    open = !open;
    if (open && btn) ax = btn.offsetLeft + btn.offsetWidth / 2;
  }
</script>

<button bind:this={btn} type="button" class="q" class:on={open} aria-label={term} aria-expanded={open} aria-controls={open ? id : undefined} onclick={toggle}>
  <Icon name="help" size={18} />
</button>
{#if open}<span class="note" {id} role="note" style:--ax="{ax}px">{@render children()}</span>{/if}

<style>
  .q {
    flex-shrink: 0; display: inline-flex; align-items: center; justify-content: center; width: 24px; height: 24px; margin-left: var(--sp-1); padding: 0;
    border: none; border-radius: var(--radius-pill); background: none; color: var(--text-3); cursor: help;
    transition: color var(--dur-fast) var(--ease), background var(--dur-fast) var(--ease);
  }
  .q:hover, .q.on { color: var(--accent-text); background: var(--accent-tint); }
  .q:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 1px; }
  .note {
    position: absolute; top: calc(100% + var(--sp-2)); left: 0; right: 0; max-width: 440px; z-index: var(--z-popover);
    display: flex; flex-direction: column; gap: var(--sp-2); padding: var(--sp-3); box-sizing: border-box;
    border-radius: var(--radius-md); border: 1px solid var(--accent-border); background: var(--glass-bg-raised);
    -webkit-backdrop-filter: var(--glass-filter); backdrop-filter: var(--glass-filter); box-shadow: var(--shadow-lg);
    font-size: var(--fs-sm); font-weight: var(--fw-medium); line-height: 1.45; color: var(--text-2); text-align: left; white-space: normal;
  }
  .note::before {
    content: ''; position: absolute; top: -6px; left: calc(var(--ax) - 5px); width: 10px; height: 10px; transform: rotate(45deg);
    background: var(--glass-bg-raised); border-left: 1px solid var(--accent-border); border-top: 1px solid var(--accent-border);
  }
  .note :global(b) { color: var(--text); font-weight: var(--fw-bold); }
  @media (pointer: coarse) {
    .q { width: 32px; height: 32px; }
  }
</style>
