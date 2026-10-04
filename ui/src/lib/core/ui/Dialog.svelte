<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import type { Snippet } from 'svelte';
  import { portal } from '$lib/core/portal';
  import { backdropDismiss } from '$lib/core/backdrop';
  import { acquireScrollLock, releaseScrollLock } from '$lib/core/scrollLock';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';
  import { isMobile } from '$lib/core/platform';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';

  interface Props {
    open: boolean;
    title?: string;
    /** CSS width; defaults to the --dialog-w token. */
    width?: string;
    /** Close when the backdrop is clicked (default true). */
    closeOnBackdrop?: boolean;
    onclose?: () => void;
    header?: Snippet;
    children: Snippet;
    footer?: Snippet;
  }

  let {
    open = $bindable(),
    title,
    width,
    closeOnBackdrop = true,
    onclose,
    header,
    children,
    footer,
  }: Props = $props();

  // On the phone a dialog is a bottom sheet, like every other choice there:
  // the same title, body and footer, the footer's buttons full-width.
  $effect(() => {
    if (!open || isMobile) return;
    acquireScrollLock();
    return () => releaseScrollLock();
  });

  function close() {
    open = false;
    onclose?.();
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && open && !isMobile) {
      e.stopPropagation();
      close();
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#if isMobile}
  <BottomSheet open={!!open} title={header ? '' : (title ?? '')} onclose={close}>
    {#if header}<div class="sheet-header">{@render header()}</div>{/if}
    <div class="sheet-body">{@render children()}</div>
    {#if footer}<div class="sheet-actions">{@render footer()}</div>{/if}
  </BottomSheet>
{:else if open}
  <div
    class="dialog-overlay"
    use:portal
    use:backdropDismiss={() => closeOnBackdrop && close()}
    role="presentation"
  >
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_noninteractive_element_interactions -->
    <div
      class="dialog"
      style={width ? `width:${width}` : undefined}
      role="dialog"
      aria-modal="true"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
    >
      {#if header}
        <div class="dialog-header">{@render header()}</div>
      {:else if title}
        <div class="dialog-header">
          <h2 class="dialog-title">{title}</h2>
          <button class="dialog-close" onclick={close} aria-label={$t('common_close')} title={$t('common_close')}>
            <Icon name="x" size={16} />
          </button>
        </div>
      {/if}

      <div class="dialog-body">
        {@render children()}
      </div>

      {#if footer}
        <div class="dialog-footer">{@render footer()}</div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .dialog-overlay {
    position: fixed;
    /* Inside the window's frame, below its title bar (desktop/WindowFrame.svelte). */
    inset: var(--overlay-inset, 0);
    border-radius: var(--overlay-radius, 0);
    background: var(--backdrop);
    -webkit-backdrop-filter: blur(2px);
    backdrop-filter: blur(2px);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--sp-4);
    z-index: var(--z-modal);
  }

  .dialog {
    display: flex;
    flex-direction: column;
    width: var(--dialog-w);
    max-width: 92vw;
    max-height: min(88vh, 100%);
    background: var(--surface-drawer);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-lg);
    overflow: hidden;
    animation: dialog-in var(--dur-base) var(--ease);
  }

  @keyframes dialog-in {
    from { opacity: 0; transform: translateY(8px) scale(0.98); }
    to   { opacity: 1; transform: none; }
  }
  @media (prefers-reduced-motion: reduce) {
    .dialog { animation: none; }
  }

  .dialog-header {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-4) var(--sp-5);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .dialog-title {
    font-size: var(--fs-md);
    font-weight: var(--fw-extrabold);
    letter-spacing: -0.3px;
    color: var(--text);
  }
  .dialog-close {
    margin-left: auto;
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
  .dialog-close:hover { background: var(--surface-hover); color: var(--text); }

  .dialog-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    scrollbar-gutter: stable;
    padding: var(--sp-5);
  }

  /* The phone's sheet. */
  .sheet-header { display: flex; align-items: center; gap: var(--sp-3); }
  .sheet-body { display: flex; flex-direction: column; gap: var(--sp-3); line-height: 1.45; }
  /* A finger does not drag a resize corner: the field grows with its text. */
  .sheet-body :global(textarea) { resize: none; }
  /* Side by side while the words fit; a long label takes a row of its own. */
  .sheet-actions { display: flex; flex-wrap: wrap; gap: var(--sp-2); padding-top: var(--sp-1); }
  .sheet-actions :global(.btn) {
    flex: 1 1 auto;
    min-width: 40%;
    min-height: var(--touch);
    justify-content: center;
    font-size: var(--fs-base);
  }

  .dialog-footer {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--sp-2);
    padding: var(--sp-3) var(--sp-5);
    border-top: 1px solid var(--border);
    flex-shrink: 0;
  }
</style>
