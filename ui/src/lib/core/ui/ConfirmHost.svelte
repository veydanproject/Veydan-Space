<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Shows the question of `ask()` (confirm.svelte.ts); each shell mounts it once.
  The desktop gets the centred confirmation Modal, the phone a bottom sheet
  with full-width buttons (Dialog presents itself as one there).
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import { isMobile } from '$lib/core/platform';
  import Modal from './Modal.svelte';
  import Dialog from './Dialog.svelte';
  import { confirmState } from './confirm.svelte';

  const q = $derived(confirmState.current);
  const danger = $derived((q?.variant ?? 'danger') === 'danger');
  const yes = $derived(q?.confirmLabel ?? $t(danger ? 'common_delete' : 'common_confirm'));
  const no = $derived(q?.cancelLabel ?? $t('common_cancel'));
</script>

{#if isMobile}
  <Dialog open={q !== null} title={q?.title ?? ''} onclose={() => confirmState.answer(false)}>
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
    onconfirm={() => confirmState.answer(true)}
    oncancel={() => confirmState.answer(false)}
  />
{/if}

<style>
  .message { margin: 0; color: var(--text-2); white-space: pre-line; }
</style>
