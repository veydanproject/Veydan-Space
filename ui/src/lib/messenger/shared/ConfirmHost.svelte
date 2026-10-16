<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Mount once per page; renders whatever `confirmStore.ask` requested. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { confirmStore } from './confirm.svelte';

  let open = $derived(confirmStore.current !== null);
</script>

<Dialog {open} width="min(420px, calc(100vw - 32px))" closable={false} onclose={() => confirmStore.answer(false)}>
  {#if confirmStore.current}
    <p class="text">{confirmStore.current.text}</p>
  {/if}
  {#snippet footer()}
    {#if confirmStore.current}
      <button class="btn btn-ghost" onclick={() => confirmStore.answer(false)}>{$t('msg_back')}</button>
      <button class="btn" class:btn-danger={confirmStore.current.danger} class:btn-primary={!confirmStore.current.danger} onclick={() => confirmStore.answer(true)}>
        {confirmStore.current.confirmLabel}
      </button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .text { margin: 0; font-size: var(--fs-sm); line-height: 1.5; color: var(--text-body); }
</style>
