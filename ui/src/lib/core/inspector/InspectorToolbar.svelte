<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { inspectorApp } from './inspector.svelte';
  import type { GridSize, ZoomLevel } from './inspector.svelte';
  import { t } from '$lib/core/i18n';
  import { toggleTheme } from '$lib/core/theme';
  import { takeScreenshot } from './screenshot';
  import InspectorCustomGrid from './InspectorCustomGrid.svelte';

  const GRID_OPTIONS: GridSize[] = [4, 8, 12, 'custom', 'off'];
  const ZOOM_OPTIONS: ZoomLevel[] = [90, 100, 110, 125];

  let viewW = $state(window.innerWidth);
  let viewH = $state(window.innerHeight);

  $effect(() => {
    function onResize() {
      viewW = window.innerWidth;
      viewH = window.innerHeight;
    }
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  });

  let presetNameInput = $state('');
  let showPresetInput = $state(false);

  function handleSavePreset() {
    if (!presetNameInput.trim()) return;
    inspectorApp.savePreset(presetNameInput.trim());
    presetNameInput = '';
    showPresetInput = false;
  }

  function handleLoadPreset(e: Event) {
    const sel = e.target as HTMLSelectElement;
    inspectorApp.loadPreset(sel.value);
  }

  function handleDeletePreset() {
    const sel = document.querySelector<HTMLSelectElement>('.preset-select');
    if (sel?.value) inspectorApp.deletePreset(sel.value);
  }

  async function handleScreenshot() {
    await takeScreenshot(inspectorApp.zoom);
  }
</script>

<div class="inspector-toolbar" data-inspector-ui>
  <!-- Viewport -->
  <span class="viewport-size">{viewW} × {viewH}</span>

  <div class="sep"></div>

  <!-- Grid -->
  <span class="label">{$t('inspector_grid')}</span>
  <div class="btn-group">
    {#each GRID_OPTIONS as opt}
      <button
        class:active={inspectorApp.gridSize === opt}
        onclick={() => (inspectorApp.gridSize = opt)}
      >
        {opt === 'off' ? $t('inspector_grid_off') : opt === 'custom' ? '⊞' : opt}
      </button>
    {/each}
  </div>

  {#if inspectorApp.gridSize !== 'off'}
    <input
      type="color"
      class="color-picker"
      bind:value={inspectorApp.gridColor}
      title="Grid color"
    />
  {/if}

  {#if inspectorApp.gridSize === 'custom'}
    <InspectorCustomGrid />
  {/if}

  <div class="sep"></div>

  <!-- Zoom -->
  <span class="label">{$t('inspector_zoom')}</span>
  <div class="btn-group">
    {#each ZOOM_OPTIONS as z}
      <button
        class:active={inspectorApp.zoom === z}
        onclick={() => (inspectorApp.zoom = z)}
      >
        {z}%
      </button>
    {/each}
  </div>

  <div class="sep"></div>

  <!-- Toggles -->
  <button
    class:active={inspectorApp.showRulers}
    title={$t('inspector_rulers')}
    onclick={() => (inspectorApp.showRulers = !inspectorApp.showRulers)}
  >
    ⊢
  </button>
  <button
    class:active={inspectorApp.showGuides}
    title={$t('inspector_guides')}
    onclick={() => (inspectorApp.showGuides = !inspectorApp.showGuides)}
  >
    ⊕
  </button>
  <button
    class:active={inspectorApp.inspectMode}
    title={$t('inspector_inspect')}
    onclick={() => (inspectorApp.inspectMode = !inspectorApp.inspectMode)}
  >
    ◎
  </button>
  <button
    class:active={inspectorApp.outlineAll}
    title={$t('inspector_outline')}
    onclick={() => (inspectorApp.outlineAll = !inspectorApp.outlineAll)}
  >
    ▣
  </button>

  <div class="sep"></div>

  <!-- Theme -->
  <button title="Toggle theme" onclick={toggleTheme}>☀</button>

  <!-- Screenshot -->
  <button title={$t('inspector_screenshot')} onclick={handleScreenshot}>⬡</button>

  <div class="sep"></div>

  <!-- Presets -->
  <span class="label">{$t('inspector_presets')}</span>
  {#if inspectorApp.presets.length > 0}
    <select class="preset-select" onchange={handleLoadPreset}>
      <option value="">—</option>
      {#each inspectorApp.presets as p}
        <option value={p.name}>{p.name}</option>
      {/each}
    </select>
    <button title={$t('inspector_preset_delete')} onclick={handleDeletePreset}>✕</button>
  {/if}
  {#if showPresetInput}
    <input
      class="preset-input"
      bind:value={presetNameInput}
      placeholder={$t('inspector_preset_name')}
      onkeydown={(e) => e.key === 'Enter' && handleSavePreset()}
    />
    <button onclick={handleSavePreset}>{$t('inspector_preset_save')}</button>
    <button onclick={() => (showPresetInput = false)}>✕</button>
  {:else}
    <button onclick={() => (showPresetInput = true)}>+</button>
  {/if}

  <div class="sep"></div>

  <!-- Close -->
  <button class="close-btn" onclick={() => inspectorApp.toggle()} title="Close inspector">✕</button>
</div>

<style>
  .inspector-toolbar {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    z-index: 8030;
    height: var(--bar-h);
    display: flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0 0.5rem;
    background: rgba(15, 23, 42, 0.94);
    border-bottom: 1px solid rgba(99, 102, 241, 0.3);
    backdrop-filter: blur(6px);
    pointer-events: auto;
    font-size: 0.72rem;
    color: #cbd5e1;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
  }

  .inspector-toolbar::-webkit-scrollbar {
    display: none;
  }

  .label {
    color: #64748b;
    white-space: nowrap;
  }

  .sep {
    width: 1px;
    height: 18px;
    background: rgba(99, 102, 241, 0.25);
    flex-shrink: 0;
    margin: 0 0.1rem;
  }

  .viewport-size {
    color: #94a3b8;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
    min-width: 80px;
  }

  .btn-group {
    display: flex;
    gap: 1px;
  }

  button {
    background: transparent;
    border: 1px solid transparent;
    border-radius: 4px;
    color: #94a3b8;
    padding: 0.15rem 0.4rem;
    cursor: pointer;
    font-size: 0.72rem;
    white-space: nowrap;
    transition: all 0.1s;
  }

  button:hover {
    background: rgba(99, 102, 241, 0.15);
    color: #e2e8f0;
  }

  button.active {
    background: rgba(99, 102, 241, 0.25);
    border-color: rgba(99, 102, 241, 0.5);
    color: #a5b4fc;
  }

  .close-btn {
    margin-left: auto;
    color: #ef4444;
    flex-shrink: 0;
  }

  .close-btn:hover {
    background: rgba(239, 68, 68, 0.15);
    color: #fca5a5;
  }

  select.preset-select {
    background: rgba(30, 41, 59, 0.9);
    border: 1px solid rgba(99, 102, 241, 0.3);
    border-radius: 4px;
    color: #cbd5e1;
    font-size: 0.72rem;
    padding: 0.1rem 0.25rem;
    cursor: pointer;
  }

  input.preset-input {
    background: rgba(30, 41, 59, 0.9);
    border: 1px solid rgba(99, 102, 241, 0.4);
    border-radius: 4px;
    color: #e2e8f0;
    font-size: 0.72rem;
    padding: 0.15rem 0.35rem;
    width: 120px;
  }

  input.preset-input:focus {
    outline: none;
    border-color: rgba(99, 102, 241, 0.7);
  }

  input.color-picker {
    width: 24px;
    height: 24px;
    padding: 1px;
    border: 1px solid rgba(99, 102, 241, 0.3);
    border-radius: 4px;
    background: transparent;
    cursor: pointer;
    flex-shrink: 0;
  }

  input.color-picker::-webkit-color-swatch-wrapper {
    padding: 0;
  }

  input.color-picker::-webkit-color-swatch {
    border: none;
    border-radius: 3px;
  }
</style>
