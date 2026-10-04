<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { messengerStore } from '../store.svelte';
  import { messengerError, type IdentityImportKind } from '../api';
  import BackupReveal from './BackupReveal.svelte';

  type Mode = 'choose' | 'create' | 'import';

  let mode = $state<Mode>('choose');
  let busy = $state(false);
  let error = $state('');

  // Create
  let password = $state('');
  let password2 = $state('');
  // Lives in the store: creating the key flips the page to the chat shell,
  // and the one-time backup must survive that.
  const created = $derived(messengerStore.pendingBackup);

  // Import
  let importKind = $state<IdentityImportKind>('nsec');
  let secret = $state('');
  let importPassword = $state('');

  const locked = $derived(!messengerStore.secretsUnlocked);

  async function create() {
    error = '';
    if (password.length < 8) { error = $t('msg_id_err_password_short'); return; }
    if (password !== password2) { error = $t('msg_id_err_password_mismatch'); return; }
    busy = true;
    try {
      await messengerStore.createIdentity(password);
      password = ''; password2 = '';
    } catch (e) { error = messengerError(e); }
    finally { busy = false; }
  }

  async function doImport() {
    error = '';
    if (!secret.trim()) { error = $t('msg_id_err_secret_empty'); return; }
    if (importKind === 'ncryptsec' && !importPassword) { error = $t('msg_id_err_password_required'); return; }
    busy = true;
    try {
      await messengerStore.importIdentity(importKind, secret, importPassword || undefined);
      secret = ''; importPassword = '';
    } catch (e) { error = messengerError(e); }
    finally { busy = false; }
  }

  function back() { mode = 'choose'; error = ''; }
</script>

<div class="card onboarding">
  {#if locked}
    <div class="locked-note">
      <Icon name="lock" size={14} />
      <span>{$t('msg_id_locked')}</span>
    </div>
  {/if}

  {#if created}
    <BackupReveal npub={created.npub} ncryptsec={created.ncryptsec} onDone={() => messengerStore.ackBackup()} />
  {:else if mode === 'choose'}
    <div class="card-title">{$t('msg_id_welcome_title')}</div>
    <p class="muted">{$t('msg_id_welcome_text')}</p>
    <div class="choice-row">
      <button class="btn btn-primary" disabled={locked} onclick={() => (mode = 'create')}>
        <Icon name="key" size={14} />{$t('msg_id_create_btn')}
      </button>
      <button class="btn btn-ghost" disabled={locked} onclick={() => (mode = 'import')}>
        <Icon name="upload" size={14} />{$t('msg_id_import_btn')}
      </button>
    </div>
  {:else if mode === 'create'}
    <div class="card-title">{$t('msg_id_create_title')}</div>
    <p class="muted">{$t('msg_id_create_text')}</p>
    <label class="field">
      <span>{$t('msg_id_backup_password')}</span>
      <input type="password" bind:value={password} autocomplete="new-password" disabled={busy} />
    </label>
    <label class="field">
      <span>{$t('msg_id_backup_password_repeat')}</span>
      <input type="password" bind:value={password2} autocomplete="new-password" disabled={busy} />
    </label>
    {#if error}<div class="error-msg">{error}</div>{/if}
    <div class="actions">
      <button class="btn btn-ghost" disabled={busy} onclick={back}>{$t('msg_back')}</button>
      <button class="btn btn-primary" disabled={busy || locked} onclick={create}>{$t('msg_id_create_confirm')}</button>
    </div>
  {:else}
    <div class="card-title">{$t('msg_id_import_title')}</div>
    <div class="kind-tabs" role="tablist">
      {#each ['nsec', 'ncryptsec', 'mnemonic'] as k}
        <button
          role="tab"
          class="kind-tab"
          class:active={importKind === k}
          aria-selected={importKind === k}
          onclick={() => { importKind = k as IdentityImportKind; error = ''; }}
        >{$t(`msg_id_kind_${k}` as 'msg_id_kind_nsec')}</button>
      {/each}
    </div>
    <p class="muted">{$t(`msg_id_kind_${importKind}_hint` as 'msg_id_kind_nsec_hint')}</p>
    <label class="field">
      <span>{$t(`msg_id_kind_${importKind}` as 'msg_id_kind_nsec')}</span>
      <textarea class="mono" rows={importKind === 'mnemonic' ? 3 : 2} bind:value={secret} spellcheck="false" disabled={busy}></textarea>
    </label>
    {#if importKind !== 'nsec'}
      <label class="field">
        <span>{importKind === 'ncryptsec' ? $t('msg_id_backup_password') : $t('msg_id_mnemonic_passphrase')}</span>
        <input type="password" bind:value={importPassword} autocomplete="off" disabled={busy} />
      </label>
    {/if}
    {#if error}<div class="error-msg">{error}</div>{/if}
    <div class="actions">
      <button class="btn btn-ghost" disabled={busy} onclick={back}>{$t('msg_back')}</button>
      <button class="btn btn-primary" disabled={busy || locked} onclick={doImport}>{$t('msg_id_import_confirm')}</button>
    </div>
  {/if}
</div>

<style>
  .onboarding { max-width: 640px; display: flex; flex-direction: column; gap: var(--sp-3); }
  .muted { color: var(--text-2); font-size: var(--fs-sm); margin: 0; }
  .locked-note {
    display: flex; align-items: center; gap: var(--sp-2);
    font-size: var(--fs-sm); color: var(--text-2);
    background: var(--surface-2); border-radius: var(--radius-sm); padding: var(--sp-2) var(--sp-3);
  }
  .choice-row, .actions { display: flex; gap: var(--sp-2); flex-wrap: wrap; }
  .actions { justify-content: flex-end; }
  .field { display: flex; flex-direction: column; gap: 6px; font-size: var(--fs-sm); }
  .field span { color: var(--text-3); font-size: var(--fs-xs); text-transform: uppercase; letter-spacing: 0.5px; }
  .field input, .field textarea {
    font: inherit; color: var(--text);
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-sm);
    padding: 8px 10px;
  }
  .field textarea { font-size: var(--fs-xs); resize: vertical; }
  .field input:focus, .field textarea:focus { outline: none; border-color: var(--accent-border); }
  .kind-tabs { display: flex; gap: var(--sp-1); }
  .kind-tab {
    background: var(--surface-2); border: 1px solid var(--border); color: var(--text-2);
    border-radius: var(--radius-sm); padding: 6px 12px; font-size: var(--fs-sm); cursor: pointer;
  }
  .kind-tab.active { background: var(--accent-bg); border-color: var(--accent-border); color: var(--accent-text); }
</style>
