<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { portal } from '$lib/core/portal';
  import { backdropDismiss } from '$lib/core/backdrop';

  interface Props {
    open: boolean;
    title: string;
    message: string;
    confirmLabel?: string;
    cancelLabel?: string;
    variant?: 'danger' | 'primary';
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
  <div class="overlay" use:portal use:backdropDismiss={oncancel} role="presentation">
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_noninteractive_element_interactions -->
    <div
      class="modal"
      onclick={(e) => e.stopPropagation()}
      role="dialog"
      aria-modal="true"
      aria-labelledby="modal-title"
      tabindex="-1"
    >
      <p id="modal-title" class="modal-title">{title}</p>
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
    background: var(--surface-drawer);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    padding: 1.25rem 1.5rem;
    box-shadow: var(--shadow-lg);
    max-width: 400px;
    width: 90%;
    display: flex;
    flex-direction: column;
    gap: 0.875rem;
  }

  .modal-title {
    font-size: var(--fs-md);
    font-weight: var(--fw-extrabold);
    letter-spacing: -0.3px;
    color: var(--text);
  }

  .modal-message {
    font-size: var(--fs-base);
    color: var(--text-2);
    line-height: 1.5;
  }

  .modal-footer {
    display: flex;
    gap: 0.5rem;
    justify-content: flex-end;
    padding-top: 0.25rem;
  }
</style>
