<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Single-choice list in a bottom sheet (theme, language, default app). -->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import BottomSheet from './BottomSheet.svelte';

  interface Props {
    open: boolean;
    title: string;
    options: { id: string; label: string; icon?: string }[];
    value: string;
    onclose: () => void;
    onpick: (id: string) => void;
  }

  let { open, title, options, value, onclose, onpick }: Props = $props();
</script>

<BottomSheet {open} {title} {onclose}>
  <div class="m-list">
    {#each options as o (o.id)}
      <button type="button" class="m-row" class:on={o.id === value} onclick={() => { onpick(o.id); onclose(); }}>
        {#if o.icon}<Icon name={o.icon} size={20} />{/if}
        <span class="m-row-label">{o.label}</span>
        {#if o.id === value}<span class="tick"><Icon name="check" size={18} /></span>{/if}
      </button>
    {/each}
  </div>
</BottomSheet>

<style>
  .m-row.on { color: var(--accent-text); font-weight: 600; }
  .tick { color: var(--accent); display: inline-flex; }
</style>
