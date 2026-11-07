<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Shows the question of `ask()` (confirm.svelte.ts); each shell mounts it once,
  and every confirmation of the app comes here (the messenger's
  `confirmStore.ask` too). The desktop gets the centred confirmation Modal,
  the phone a bottom sheet with full-width buttons (Dialog presents itself as
  one there): the same title, the same red tile of a dangerous question.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import { isMobile } from '$lib/core/platform';
  import Icon from '$lib/core/Icon.svelte';
  import Modal from './Modal.svelte';
  import Dialog from './Dialog.svelte';
  import { confirmState } from './confirm.svelte';

  const q = $derived(confirmState.current);
  const danger = $derived((q?.variant ?? 'danger') === 'danger');
  const icon = $derived(q?.icon ?? 'alert-triangle');
  const yes = $derived(q?.confirmLabel ?? $t(danger ? 'common_delete' : 'common_confirm'));
  const no = $derived(q?.cancelLabel ?? $t('common_cancel'));
</script>

{#if isMobile}
  <Dialog open={q !== null} label={q?.title ?? ''} closable={false} onclose={() => confirmState.answer(false)}>
    {#snippet header()}
      {#if danger}<span class="confirm-tile" aria-hidden="true"><Icon name={icon} size={20} /></span>{/if}
      <h2 class="title">{q?.title ?? ''}</h2>
    {/snippet}
    {#if q?.message}<p class="message">{q.message}</p>{/if}
    {#snippet footer()}
      <button type="button" class="btn btn-ghost" onclick={() => confirmState.answer(false)}>{no}</button>
      <button
        type="button"
        class="btn"
        class:btn-danger={danger}
        class:btn-primary={!danger}
        onclick={() => confirmState.answer(true)}
      >
        {yes}
      </button>
    {/snippet}
  </Dialog>
{:else}
  <Modal
    open={q !== null}
    title={q?.title ?? ''}
    message={q?.message ?? ''}
    confirmLabel={yes}
    cancelLabel={no}
    variant={danger ? 'danger' : 'primary'}
    {icon}
    onconfirm={() => confirmState.answer(true)}
    oncancel={() => confirmState.answer(false)}
  />
{/if}

<style>
  /* The tile of Modal.svelte, in the phone's sheet. */
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
  .title {
    flex: 1;
    min-width: 0;
    margin: 0;
    font-size: var(--fs-lg);
    font-weight: var(--fw-bold);
    line-height: var(--lh-tight);
    letter-spacing: var(--ls-tight);
    color: var(--text);
  }
  .message { margin: 0; color: var(--text-2); white-space: pre-line; }
</style>
