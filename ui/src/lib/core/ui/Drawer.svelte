<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import type { Snippet } from 'svelte';
  import { portal } from '$lib/core/portal';
  import { backdropDismiss } from '$lib/core/backdrop';
  import { acquireScrollLock, releaseScrollLock } from '$lib/core/scrollLock';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';

  interface Props {
    open: boolean;
    title?: string;
    /** CSS width; defaults to the --drawer-w token. Use --drawer-w-lg for wide. */
    width?: string;
    side?: 'right' | 'left';
    closeOnBackdrop?: boolean;
    onclose?: () => void;
    /** Small marker rendered right after the title. */
    titleBadge?: Snippet;
    /** Extra controls rendered in the header, before the close button. */
    actions?: Snippet;
    /** Fixed region between the header and the scrolling body (e.g. a tab bar or toolbar). */
    subheader?: Snippet;
    children: Snippet;
    footer?: Snippet;
  }

  let {
    open = $bindable(),
    title,
    width,
    side = 'right',
    closeOnBackdrop = true,
    onclose,
    titleBadge,
    actions,
    subheader,
    children,
    footer,
  }: Props = $props();

  $effect(() => {
    if (!open) return;
    acquireScrollLock();
    return () => releaseScrollLock();
  });

  function close() {
    open = false;
    onclose?.();
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && open) {
      e.stopPropagation();
      close();
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#if open}
  <div
    class="drawer-backdrop"
    use:portal
    use:backdropDismiss={() => closeOnBackdrop && close()}
    role="presentation"
  >
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_noninteractive_element_interactions -->
    <div
      class="drawer {side}"
      style={width ? `width:${width}` : undefined}
      role="dialog"
      aria-modal="true"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
    >
      {#if title || actions}
        <header class="drawer-header">
          {#if title}<h2 class="drawer-title">{title}</h2>{/if}
          {@render titleBadge?.()}
          <div class="drawer-actions">
            {@render actions?.()}
            <button class="drawer-close" onclick={close} aria-label={$t('common_close')} title={$t('common_close')}>
              <Icon name="x" size={16} />
            </button>
          </div>
        </header>
      {/if}

      {#if subheader}
        <div class="drawer-subheader">{@render subheader()}</div>
      {/if}

      <div class="drawer-body">
        {@render children()}
      </div>

      {#if footer}
        <footer class="drawer-footer">{@render footer()}</footer>
      {/if}
    </div>
  </div>
{/if}

<style>
  .drawer-backdrop {
    position: fixed;
    /* Inside the window's frame, below its title bar (desktop/WindowFrame.svelte). */
    inset: var(--overlay-inset, 0);
    border-radius: var(--overlay-radius, 0);
    overflow: hidden;
    background: var(--backdrop);
    -webkit-backdrop-filter: blur(2px);
    backdrop-filter: blur(2px);
    z-index: var(--z-drawer);
    display: flex;
    animation: backdrop-in var(--dur-base) ease;
  }
  .drawer-backdrop:has(.left) { justify-content: flex-start; }
  .drawer-backdrop:has(.right) { justify-content: flex-end; }

  .drawer {
    display: flex;
    flex-direction: column;
    width: var(--drawer-w);
    max-width: 92vw;
    height: 100%;
    background: var(--surface-drawer);
    box-shadow: var(--shadow-drawer);
  }
  .drawer.right { border-left: 1px solid var(--border); animation: drawer-in-right var(--dur-drawer) var(--ease-drawer); }
  .drawer.left  { border-right: 1px solid var(--border); animation: drawer-in-left  var(--dur-drawer) var(--ease-drawer); }

  @keyframes backdrop-in { from { opacity: 0; } to { opacity: 1; } }
  @keyframes drawer-in-right { from { transform: translateX(30px); opacity: 0.4; } to { transform: none; opacity: 1; } }
  @keyframes drawer-in-left  { from { transform: translateX(-30px); opacity: 0.4; } to { transform: none; opacity: 1; } }
  @media (prefers-reduced-motion: reduce) {
    .drawer { animation: none; }
  }

  .drawer-header {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-5) var(--sp-5);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .drawer-title {
    font-size: 1.15rem;
    font-weight: var(--fw-extrabold);
    letter-spacing: -0.3px;
    color: var(--text);
  }
  .drawer-actions {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin-left: auto;
  }
  .drawer-close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 34px;
    height: 34px;
    border-radius: 9px;
    background: transparent;
    color: var(--text-2);
    border: none;
    flex-shrink: 0;
  }
  .drawer-close:hover { background: var(--surface-hover); color: var(--text); }

  .drawer-subheader {
    flex-shrink: 0;
  }

  .drawer-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    scrollbar-gutter: stable;
    padding: var(--sp-4);
  }

  .drawer-footer {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--sp-2);
    padding: var(--sp-4) var(--sp-5);
    border-top: 1px solid var(--border);
    background: var(--surface-drawer-footer);
    flex-shrink: 0;
  }
</style>
