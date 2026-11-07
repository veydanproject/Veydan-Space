<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The desktop's confirmation (core/ui/ConfirmHost.svelte, the only user):
  the title, the text under it, «Отмена» on the left and the verb on the
  right; a dangerous question has the red tile with its action's icon
  beside the title.
-->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { portal } from '$lib/core/portal';
  import { backdropDismiss } from '$lib/core/backdrop';
  import { backdropIn, backdropOut, dialogIn, dialogOut } from './motion';

  interface Props {
    open: boolean;
    title: string;
    message: string;
    confirmLabel?: string;
    cancelLabel?: string;
    variant?: 'danger' | 'primary';
    /** The icon of a dangerous question's tile. */
    icon?: string;
    onconfirm: () => void;
    oncancel: () => void;
  }

  let {
    open,
    title,
    message,
    confirmLabel = 'OK',
    cancelLabel = 'Cancel',
    variant = 'danger',
    icon = 'alert-triangle',
    onconfirm,
    oncancel,
  }: Props = $props();

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && open) oncancel();
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if open}
  <!-- Portalled to body so it stacks above drawers, which live on body too -->
  <div class="overlay" in:backdropIn out:backdropOut use:portal use:backdropDismiss={oncancel} role="presentation">
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_noninteractive_element_interactions -->
    <div
      class="modal on-glass"
      in:dialogIn
      out:dialogOut
      onclick={(e) => e.stopPropagation()}
      role="dialog"
      aria-modal="true"
      aria-labelledby="modal-title"
      tabindex="-1"
    >
      <div class="modal-head">
        {#if variant === 'danger'}<span class="confirm-tile" aria-hidden="true"><Icon name={icon} size={20} /></span>{/if}
        <h2 id="modal-title" class="modal-title">{title}</h2>
      </div>
      {#if message}<p class="modal-message">{message}</p>{/if}
      <div class="modal-footer">
        <button class="btn btn-ghost" onclick={oncancel}>{cancelLabel}</button>
        <button
          class="btn"
          class:btn-danger={variant === 'danger'}
          class:btn-primary={variant === 'primary'}
          onclick={onconfirm}
        >
          {confirmLabel}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    /* Inside the window's frame, below its title bar (desktop/WindowFrame.svelte). */
    inset: var(--overlay-inset, 0);
    border-radius: var(--overlay-radius, 0);
    background: var(--backdrop);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: var(--z-modal);
    -webkit-backdrop-filter: blur(2px);
    backdrop-filter: blur(2px);
  }

  .modal {
    background: var(--glass-bg);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-card);
    padding: var(--sp-6);
    box-shadow: var(--glass-highlight), var(--shadow-lg);
    max-width: var(--dialog-w);
    width: 90%;
    display: flex;
    flex-direction: column;
    gap: 0.875rem;
    -webkit-backdrop-filter: var(--glass-filter);
    backdrop-filter: var(--glass-filter);
  }

  .modal-head {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
  }

  /* A dangerous question's tile: 40 × 40, red, the action's icon. */
  .confirm-tile {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 40px;
    height: 40px;
    border-radius: var(--radius-md);
    background: var(--danger-bg);
    color: var(--danger-text);
  }

  .modal-title {
    flex: 1;
    min-width: 0;
    margin: 0;
    font-size: var(--fs-lg);
    font-weight: var(--fw-bold);
    line-height: var(--lh-tight);
    letter-spacing: var(--ls-tight);
    color: var(--text);
  }

  .modal-message {
    font-size: var(--fs-base);
    color: var(--text-2);
    line-height: var(--lh-base);
    white-space: pre-line;
  }

  .modal-footer {
    display: flex;
    gap: 0.5rem;
    justify-content: flex-end;
    padding-top: 0.25rem;
  }
</style>
