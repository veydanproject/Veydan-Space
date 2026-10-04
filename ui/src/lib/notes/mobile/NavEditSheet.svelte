<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { NAV_COLORS } from '$lib/core/mobile/nav-colors';
  import { t } from '$lib/core/mobile/i18n';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';

  interface Props {
    open: boolean;
    title: string;
    name: string;
    color: string;
    placeholder: string;
    saving?: boolean;
    onclose: () => void;
    onsave: (name: string, color: string) => void;
  }

  let { open, title, name = $bindable(), color = $bindable(), placeholder, saving = false, onclose, onsave }: Props = $props();
</script>

<BottomSheet {open} {title} {onclose}>
  <input type="text" class="name" bind:value={name} {placeholder} onkeydown={(e) => { if (e.key === 'Enter' && name.trim()) onsave(name.trim(), color); }} />
  <div class="swatches">
    {#each NAV_COLORS as c (c)}
      <button type="button" class="swatch" class:on={color === c} style:background={c} aria-label={c} onclick={() => (color = c)}></button>
    {/each}
  </div>
  <button type="button" class="m-btn-grad" disabled={!name.trim() || saving} onclick={() => onsave(name.trim(), color)}>
    {$t('common_save')}
  </button>
</BottomSheet>

<style>
  .name {
    border: 0;
    border-radius: 12px;
    background: var(--m-field);
    padding: 0 14px;
  }
  .swatches { display: flex; gap: 12px; flex-wrap: wrap; padding: var(--sp-1) 3px; }
  .swatch {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    border: 3px solid transparent;
    padding: 0;
    box-shadow: 0 0 0 2px var(--m-card-border);
  }
  .swatch.on { border-color: var(--m-sheet); box-shadow: 0 0 0 2px var(--text); }
</style>
