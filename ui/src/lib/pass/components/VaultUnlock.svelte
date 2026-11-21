<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';
  import { t } from '$lib/core/i18n';
  import { isMobile } from '$lib/core/platform';
  import { appLock } from '$lib/core/lock/store.svelte';

  interface Props {
    open: boolean;
    /** Runs after the vault password is accepted. */
    onunlocked: () => void;
  }

  let { open = $bindable(false), onunlocked }: Props = $props();

  let password = $state('');
  let wrong = $state(false);
  let busy = $state(false);

  $effect(() => {
    if (!open) return;
    password = '';
    wrong = false;
  });

  function close() {
    open = false;
  }

  async function submit() {
    if (!password || busy) return;
    busy = true;
    wrong = false;
    try {
      await appLock.unlock(password);
      close();
      onunlocked();
    } catch {
      wrong = true;
      password = '';
    } finally {
      busy = false;
    }
  }
</script>

<!-- A phone's sheet: the field under its label, focused; the verb in the sheet's foot (it submits the form by its id). -->
{#snippet form()}
  <form id="vault-unlock" class="form" onsubmit={(e) => { e.preventDefault(); void submit(); }}>
    {#if isMobile}<label class="form-label" for="vault-unlock-pw">{$t('lock_password')}</label>{/if}
    <input
      id="vault-unlock-pw"
      class:pin={appLock.status.kind === 'pin'}
      type={appLock.status.kind === 'pin' ? 'text' : 'password'}
      inputmode={appLock.status.kind === 'pin' ? 'numeric' : 'text'}
      pattern={appLock.status.kind === 'pin' ? '[0-9]*' : undefined}
      bind:value={password}
      aria-invalid={wrong || undefined}
      placeholder={$t('lock_password')}
      autocomplete="off"
      autocapitalize="off"
      spellcheck="false"
      {@attach (el: HTMLInputElement) => { if (isMobile) el.focus(); }}
    />
    {#if wrong}<p class="error-msg" role="alert">{$t('lock_wrong')}</p>{/if}
    {#if !isMobile}<button class="btn btn-primary" type="submit" disabled={!password || busy}>{$t('lock_unlock')}</button>{/if}
  </form>
{/snippet}

{#if isMobile}
  <BottomSheet {open} title={$t('lock_title')} onclose={close}>
    {@render form()}
    {#snippet footer()}
      <button class="btn btn-primary" type="submit" form="vault-unlock" disabled={!password || busy}>{$t('lock_unlock')}</button>
    {/snippet}
  </BottomSheet>
{:else}
  <Dialog bind:open title={$t('lock_title')} onclose={close} width="360px">
    {@render form()}
  </Dialog>
{/if}

<style>
  .form { display: flex; flex-direction: column; gap: var(--sp-3); }
  .form-label { margin-bottom: calc(var(--sp-1) - var(--sp-3)); }
  input {
    width: 100%;
    min-height: var(--field-h);
    padding: 0 var(--sp-2-5);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface-3);
    color: var(--text);
  }
  .error-msg { margin: 0; }
  input.pin { -webkit-text-security: disc; }
</style>
