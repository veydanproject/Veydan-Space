<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { inspectorApp } from './inspector.svelte';
  import { t } from '$lib/core/i18n';
</script>

{#if inspectorApp.gridSize === 'custom'}
  <div class="custom-grid-panel" data-inspector-ui>
    <label>
      <span>{$t('inspector_columns')}</span>
      <input
        type="number"
        min="1"
        max="24"
        bind:value={inspectorApp.customGrid.columns}
      />
    </label>
    <label>
      <span>{$t('inspector_gutter')} px</span>
      <input
        type="number"
        min="0"
        max="64"
        bind:value={inspectorApp.customGrid.gutter}
      />
    </label>
    <label>
      <span>{$t('inspector_margin')} px</span>
      <input
        type="number"
        min="0"
        max="128"
        bind:value={inspectorApp.customGrid.margin}
      />
    </label>
  </div>
{/if}

<style>
  .custom-grid-panel {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    pointer-events: auto;
  }

  label {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    font-size: 0.72rem;
    color: var(--color-text-muted);
  }

  input {
    width: 3.5rem;
    padding: 0.15rem 0.3rem;
    background: var(--color-bg-soft);
    border: 1px solid var(--color-border-soft);
    border-radius: 4px;
    color: var(--color-text-main);
    font-size: 0.72rem;
    text-align: center;
  }

  input:focus {
    outline: 1px solid var(--color-brand-500);
  }
</style>
