<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The window of a desktop product. With client-side decorations (Linux) the
  OS draws nothing: the webview is transparent and the app lives inside a
  rounded, shadowed panel with a gutter for the drop shadow, under a title bar
  with the window controls; the frame is dropped when the window is
  maximized. On Windows and macOS this is the plain full-window panel.

  Everything a product shows goes inside: the shell, and the screen shown
  instead of it when the app cannot start. Full-window overlays (Dialog,
  Drawer, Modal, the command palette) do not know the frame; it publishes
  `--overlay-inset` and `--overlay-radius` on <html>, and their backdrops
  stay inside the panel and below the title bar, which keeps working.
-->
<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import type { Window } from '@tauri-apps/api/window';
  import { FRAME, isCsd } from '$lib/core/desktop/csd';
  import WindowControls from '$lib/core/desktop/WindowControls.svelte';
  import ResizeHandles from '$lib/core/desktop/ResizeHandles.svelte';

  interface Props {
    /** The name in the title bar. */
    title: string;
    /** After the name: the way back from a screen outside the navigation. */
    lead?: Snippet;
    /** Buttons before the window controls. */
    tools?: Snippet;
    children: Snippet;
  }

  let { title, lead, tools, children }: Props = $props();

  const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

  // The window is cached so startDragging() fires synchronously inside the
  // mousedown (Wayland needs the live grab).
  let appWindow: Window | null = null;
  let maximized = $state(false);
  const NON_DRAG = 'button, a, input, select, textarea, [data-no-drag]';

  function onTitlebarMouseDown(e: MouseEvent) {
    if (e.button !== 0 || !appWindow) return;
    if ((e.target as HTMLElement).closest(NON_DRAG)) return;
    void appWindow.startDragging();
  }
  function onTitlebarDblClick(e: MouseEvent) {
    if (!appWindow) return;
    if ((e.target as HTMLElement).closest(NON_DRAG)) return;
    void appWindow.toggleMaximize();
  }

  onMount(() => {
    if (!isTauri) return;
    let unlistenMax: (() => void) | undefined;
    let gone = false;
    import('@tauri-apps/api/window')
      .then(async ({ getCurrentWindow }) => {
        const win = getCurrentWindow();
        appWindow = win;
        try {
          maximized = await win.isMaximized();
          const stop = await win.onResized(async () => {
            try { maximized = await win.isMaximized(); } catch {}
          });
          if (gone) stop();
          else unlistenMax = stop;
        } catch {}
      })
      .catch(() => {});
    return () => {
      gone = true;
      unlistenMax?.();
    };
  });

  // Where a full-window overlay may be: inside the panel, below the title bar.
  $effect(() => {
    if (!isCsd) return;
    const root = document.documentElement;
    const gutter = maximized ? 0 : FRAME.gutter;
    const radius = maximized ? 0 : FRAME.radius;
    root.style.setProperty(
      '--overlay-inset',
      `calc(${gutter + FRAME.titlebar}px + var(--inspector-bar-height, 0px)) ${gutter}px ${gutter}px`,
    );
    root.style.setProperty('--overlay-radius', `0 0 ${radius}px ${radius}px`);
    return () => {
      root.style.removeProperty('--overlay-inset');
      root.style.removeProperty('--overlay-radius');
    };
  });
</script>

<div class="app-frame" class:csd={isCsd} class:maximized>
  <div class="layout">
    {#if isCsd}
      <div class="titlebar">
        <div class="titlebar-title">
          <img src="/logo.png" alt="" class="titlebar-logo" />
          <span>{title}</span>
        </div>
        {@render lead?.()}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="titlebar-drag" onmousedown={onTitlebarMouseDown} ondblclick={onTitlebarDblClick}></div>
        {@render tools?.()}
        <WindowControls />
      </div>
    {/if}
    {@render children()}
  </div>
</div>

{#if isCsd}
  <ResizeHandles />
{/if}

<style>
  /* The OS draws no decoration (decorations: false): the window is
     transparent and the panel is the window the user sees. */
  :global(html),
  :global(body) {
    background: transparent !important;
    overflow: hidden;
  }

  .app-frame {
    position: fixed;
    inset: 0;
    background: var(--bg);
  }
  /* CSD (Linux): rounded, shadowed panel with a gutter for the drop shadow. */
  .app-frame.csd {
    inset: 9px;
    border-radius: 11px;
    overflow: hidden;
    box-shadow:
      0 0 0 1px var(--border),
      0 14px 44px rgba(0, 0, 0, 0.5);
  }
  .app-frame.csd.maximized {
    inset: 0;
    border-radius: 0;
    box-shadow: none;
  }

  .layout {
    position: relative;
    display: flex;
    flex-direction: column;
    height: calc(100% - var(--inspector-bar-height, 0px));
    margin-top: var(--inspector-bar-height, 0px);
  }

  /* The window's own bar, above everything the product shows. */
  .titlebar {
    display: flex;
    align-items: stretch;
    height: 34px;
    flex-shrink: 0;
    padding-left: 12px;
    background: var(--chrome);
    border-bottom: 1px solid var(--border);
  }
  .titlebar-title {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 0.78rem;
    font-weight: var(--fw-semibold);
    color: var(--text-2);
    letter-spacing: 0.1px;
    -webkit-user-select: none;
    user-select: none;
  }
  .titlebar-logo {
    width: 18px;
    height: 18px;
    object-fit: contain;
  }
  /* Empty flexible middle — the primary drag area. */
  .titlebar-drag {
    flex: 1;
  }

  /* A button of the title bar (`tools`, `lead`): flat and full-height, like the window controls. */
  .titlebar :global(.titlebar-btn) {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    align-self: stretch;
    min-width: 42px;
    padding: 0 12px;
    border: none;
    border-radius: 0;
    background: transparent;
    color: var(--text-soft);
    font-size: 0.78rem;
    font-weight: var(--fw-semibold);
    cursor: pointer;
    transition: background 0.12s, color 0.12s;
  }
  .titlebar :global(.titlebar-btn:hover) {
    background: var(--surface-hover);
    color: var(--text);
  }
  .titlebar :global(.titlebar-btn.active) {
    color: var(--accent-text);
  }
  .titlebar :global(.titlebar-btn:focus-visible) {
    outline: 2px solid var(--accent-border);
    outline-offset: -2px;
  }
</style>
