<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Mount once per page; renders whatever `confirmStore.ask` requested. A
  dangerous question shows the warning tile beside its text; «Назад | verb»
  are pinned in the footer, the verb red when it is dangerous (design book,
  «Чаты · окна и подтверждения»).
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { confirmStore } from './confirm.svelte';

  let open = $derived(confirmStore.current !== null);
</script>

<Dialog {open} width="min(420px, calc(100vw - 32px))" closable={false} onclose={() => confirmStore.answer(false)}>
  {#if confirmStore.current}
    <div class="question">
      {#if confirmStore.current.danger}<span class="warn" aria-hidden="true"><Icon name="alert-triangle" size={20} /></span>{/if}
      <p class="text">{confirmStore.current.text}</p>
    </div>
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
  .question { display: flex; align-items: flex-start; gap: var(--sp-3); }
  .warn {
    flex-shrink: 0; display: inline-flex; align-items: center; justify-content: center;
    width: 40px; height: 40px; border-radius: var(--radius-md); background: var(--danger-bg); color: var(--danger-text);
  }
  .text { flex: 1; min-width: 0; margin: 0; font-size: var(--fs-body); line-height: 1.5; color: var(--text); }
</style>
