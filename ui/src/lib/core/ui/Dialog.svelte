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
  import { backdropIn, backdropOut, dialogIn, dialogOut } from './motion';

  interface Props {
    open: boolean;
    title?: string;
    /** CSS width; defaults to the --dialog-w token. */
    width?: string;
    /** Close when the backdrop is clicked (default true). */
    closeOnBackdrop?: boolean;
    /** The phone's sheet keeps its × (default true); a question whose footer has «Отмена» passes false. */
    closable?: boolean;
    /** The phone's sheet can be pulled up to the whole screen (BottomSheet `expandable`). */
    expandable?: boolean;
    /** The phone's sheet without a title: what a screen reader calls it. */
    label?: string;
    onclose?: () => void;
    header?: Snippet;
    /** Beside the title, in the same head (a «?» of what the title means); a phone's sheet keeps it in its head too. */
    titleAside?: Snippet;
    children: Snippet;
    footer?: Snippet;
  }

  let {
    open = $bindable(),
    title,
    width,
    closeOnBackdrop = true,
    closable = true,
    expandable = false,
    label,
    onclose,
    header,
    titleAside,
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

<!-- The phone's footer goes to the sheet's own foot, outside its scrolling body: the buttons stay in sight. -->
{#snippet sheetFooter()}<div class="sheet-actions">{@render footer?.()}</div>{/snippet}

{#if isMobile}
  <BottomSheet open={!!open} title={header ? '' : (title ?? '')} onclose={close} {closable} {titleAside} {expandable} {label} footer={footer ? sheetFooter : undefined}>
    {#if header}<div class="sheet-header">{@render header()}</div>{/if}
    <div class="sheet-body" class:grow={expandable}>{@render children()}</div>
  </BottomSheet>
{:else if open}
  <div
    class="dialog-overlay"
    in:backdropIn
    out:backdropOut
    use:portal
    use:backdropDismiss={() => closeOnBackdrop && close()}
    role="presentation"
  >
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_noninteractive_element_interactions -->
    <div
      class="dialog"
      in:dialogIn
      out:dialogOut
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
          <!-- The «?» right after the title, outside the heading: its note is not read as part of the title. -->
          <div class="dialog-title-row"><h2 class="dialog-title">{title}</h2>{@render titleAside?.()}</div>
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
    background: var(--glass-bg);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-card);
    box-shadow: var(--glass-highlight), var(--shadow-lg);
    overflow: hidden;
    -webkit-backdrop-filter: var(--glass-filter);
    backdrop-filter: var(--glass-filter);
  }


  .dialog-header {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-height: 64px;
    padding: 0 var(--sp-3) 0 var(--sp-6);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  /* The title and its «?»: the row a note opens under. */
  .dialog-title-row {
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
  }
  .dialog-title {
    min-width: 0;
    font-size: var(--fs-lg);
    font-weight: var(--fw-bold);
    line-height: 1.35;
    color: var(--text);
  }
  .dialog-close {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 40px;
    height: 40px;
    padding: 0;
    border-radius: var(--radius);
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
    padding: var(--sp-5) var(--sp-6);
  }

  /* The phone's sheet. */
  .sheet-header { display: flex; align-items: center; gap: var(--sp-3); }
  .sheet-body { display: flex; flex-direction: column; gap: var(--sp-3); line-height: 1.45; }
  /* A sheet that can be pulled up fills the screen it grew to. */
  .sheet-body.grow { flex: 1 0 auto; }
  /* A finger does not drag a resize corner: the field grows with its text. */
  .sheet-body :global(textarea) { resize: none; }
  /* Stacked, the whole width, 48 high, the verb on top: the footer reads
     «Отмена, verb» like the desktop's, the sheet turns it over (design kit). */
  .sheet-actions { display: flex; flex-direction: column-reverse; gap: var(--sp-2); }
  .sheet-actions :global(.btn) {
    width: 100%;
    min-height: 48px;
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
