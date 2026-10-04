<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Custom window controls (minimize / maximize-restore / close) for the
  client-side-decorated window (tauri.conf.json `decorations: false`). Server-
  side decorations are disabled to avoid the KWin hide()/show() decoration bug
  on Wayland, so the window chrome lives in the web UI instead.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/core/api';
  import { t } from '$lib/core/i18n';
  import type { Window } from '@tauri-apps/api/window';

  const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

  let win: Window | null = null;
  let maximized = $state(false);

  onMount(() => {
    if (!isTauri) return;
    let unlisten: (() => void) | undefined;
    (async () => {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      const w = getCurrentWindow();
      win = w;
      try {
        maximized = await w.isMaximized();
        unlisten = await w.onResized(async () => {
          try {
            maximized = await w.isMaximized();
          } catch {}
        });
      } catch {}
    })();
    return () => unlisten?.();
  });
</script>

{#if isTauri}
  <div class="win-controls">
    <button class="wc" onclick={() => api.settings.windowMinimize()} aria-label={$t('window_minimize')} title={$t('window_minimize')}>
      <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
        <line x1="3" y1="7.5" x2="11" y2="7.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
      </svg>
    </button>
    <button
      class="wc"
      onclick={() => win?.toggleMaximize()}
      aria-label={maximized ? $t('window_restore') : $t('window_maximize')}
      title={maximized ? $t('window_restore') : $t('window_maximize')}
    >
      {#if maximized}
        <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
          <rect x="3" y="5" width="6" height="6" rx="1" fill="none" stroke="currentColor" stroke-width="1.5" />
          <path d="M5.4 5 V3 H11 V8.6 H9" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" />
        </svg>
      {:else}
        <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
          <rect x="3" y="3" width="8" height="8" rx="1.2" fill="none" stroke="currentColor" stroke-width="1.5" />
        </svg>
      {/if}
    </button>
    <button class="wc wc-close" onclick={() => win?.close()} aria-label={$t('common_close')} title={$t('common_close')}>
      <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
        <line x1="3.4" y1="3.4" x2="10.6" y2="10.6" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
        <line x1="10.6" y1="3.4" x2="3.4" y2="10.6" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
      </svg>
    </button>
  </div>
{/if}

<style>
  /* Flat, full-height controls that sit flush in the titlebar (Windows-style).
     The rounded window corner clips the close button's hover cleanly. */
  .win-controls {
    display: flex;
    align-self: stretch;
  }
  .wc {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 46px;
    align-self: stretch;
    border: none;
    background: transparent;
    color: var(--text-soft);
    cursor: pointer;
    transition: background 0.12s, color 0.12s;
  }
  .wc:hover {
    background: var(--surface-hover);
    color: var(--text);
  }
  .wc-close:hover {
    background: #e5484d;
    color: #fff;
  }
</style>
