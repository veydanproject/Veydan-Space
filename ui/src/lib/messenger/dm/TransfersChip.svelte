<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Files on their way, in the chat's header: how many go up, how many come
  down, how many failed. Hidden while there are none; a press lists them.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import TransfersPanel from '../media/TransfersPanel.svelte';
  import { transferStore } from '../media/transferStore.svelte';

  interface Props {
    /** A phone screen: the list is a sheet. */
    phone?: boolean;
  }
  let { phone = false }: Props = $props();

  const counts = $derived(transferStore.counts);
  const any = $derived(counts.up + counts.down + counts.failed > 0);
  let open = $state(false);
  let anchor = $state<DOMRect | null>(null);

  onMount(() => { transferStore.load(); });

  function toggle(e: MouseEvent) {
    anchor = (e.currentTarget as HTMLElement).getBoundingClientRect();
    open = !open;
  }
</script>

{#if any}
  <button class="xfer" onclick={toggle} aria-haspopup="dialog" aria-expanded={open}
    aria-label={$t('msg_xfer_chip', { up: String(counts.up), down: String(counts.down), failed: String(counts.failed) })}
    title={$t('msg_xfer_title')}>
    <span class="wide">
      {#if counts.up}<span class="n"><Icon name="arrow-up" size={14} />{counts.up}</span>{/if}
      {#if counts.down}<span class="n"><Icon name="arrow-down" size={14} />{counts.down}</span>{/if}
      {#if counts.failed}<span class="n failed"><Icon name="alert-triangle" size={14} />{counts.failed}</span>{/if}
    </span>
    <!-- A narrow header has room for less: those on their way in one number, then the failed ones. -->
    <span class="narrow">
      {#if counts.up + counts.down}<span class="n"><Icon name={counts.up >= counts.down ? 'arrow-up' : 'arrow-down'} size={14} />{counts.up + counts.down}</span>{/if}
      {#if counts.failed}<span class="n failed"><Icon name="alert-triangle" size={14} />{counts.failed}</span>{/if}
    </span>
  </button>
{/if}
<TransfersPanel bind:open {anchor} sheet={phone} />

<style>
  /* A quiet pill of the head (as the feed's): the counts in the second text, the failed ones in the danger's. */
  .xfer {
    display: inline-flex; align-items: center; gap: var(--sp-1); flex-shrink: 0; height: 28px; padding: 0 var(--sp-3); cursor: pointer;
    border: 1px solid var(--border); border-radius: var(--radius-pill); background: var(--surface-2);
    color: var(--text-2); font: inherit; font-size: var(--fs-xs); font-weight: var(--fw-semibold); font-variant-numeric: tabular-nums;
    transition: background var(--dur-fast), color var(--dur-fast);
  }
  .xfer:hover { background: var(--surface-hover); color: var(--text); }
  .xfer:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  .n { display: inline-flex; align-items: center; gap: 2px; }
  .n.failed { color: var(--danger-text); }
  .wide { display: inline-flex; align-items: center; gap: 6px; }
  .narrow { display: none; }
  @media (pointer: coarse) { .xfer { height: 36px; } }
  @media (max-width: 520px) {
    .wide { display: none; }
    .narrow { display: inline-flex; align-items: center; gap: 5px; }
  }
</style>
