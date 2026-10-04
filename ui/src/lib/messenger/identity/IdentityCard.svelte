<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { t } from '$lib/core/i18n';
  import { stamp } from '../shared/time';
  import Icon from '$lib/core/Icon.svelte';
  import { messengerStore } from '../store.svelte';
  import { messengerError, type MessengerIdentity } from '../api';
  import CopyField from './CopyField.svelte';

  interface Props { identity: MessengerIdentity }
  let { identity }: Props = $props();

  type Panel = 'none' | 'export' | 'delete';
  let panel = $state<Panel>('none');
  let busy = $state(false);
  let error = $state('');
  let exportPassword = $state('');
  let exported = $state('');

  const locked = $derived(!messengerStore.secretsUnlocked);

  function open(p: Panel) { panel = p; error = ''; exported = ''; exportPassword = ''; }

  async function doExport() {
    error = '';
    if (exportPassword.length < 8) { error = $t('msg_id_err_password_short'); return; }
    busy = true;
    try { exported = await messengerStore.exportIdentity(exportPassword); exportPassword = ''; }
    catch (e) { error = messengerError(e); }
    finally { busy = false; }
  }

  async function doDelete() {
    error = '';
    busy = true;
    try { await messengerStore.deleteIdentity(); panel = 'none'; }
    catch (e) { error = messengerError(e); }
    finally { busy = false; }
  }
</script>

<div class="card identity">
  <div class="card-title"><Icon name="user" size={16} /> {$t('msg_id_title')}</div>
  <CopyField label={$t('msg_id_npub')} value={identity.npub} />
  <div class="meta">{$t('msg_id_created', { date: stamp(identity.created_at) })}</div>

  {#if locked}
    <div class="locked-note"><Icon name="lock" size={14} /><span>{$t('msg_id_locked')}</span></div>
  {/if}

  <div class="actions">
    <button class="btn btn-ghost" disabled={locked} onclick={() => open(panel === 'export' ? 'none' : 'export')}>
      <Icon name="key" size={14} />{$t('msg_id_export_btn')}
    </button>
    <button class="btn btn-ghost danger" disabled={locked} onclick={() => open(panel === 'delete' ? 'none' : 'delete')}>
      <Icon name="trash-2" size={14} />{$t('msg_id_delete_btn')}
    </button>
  </div>

  {#if panel === 'export'}
    <div class="panel">
      {#if exported}
        <CopyField label={$t('msg_id_ncryptsec')} value={exported} mono />
        <p class="muted">{$t('msg_id_export_hint_done')}</p>
      {:else}
        <p class="muted">{$t('msg_id_export_text')}</p>
        <label class="field">
          <span>{$t('msg_id_backup_password')}</span>
          <input type="password" bind:value={exportPassword} autocomplete="new-password" disabled={busy} />
        </label>
        {#if error}<div class="error-msg">{error}</div>{/if}
        <div class="actions end">
          <button class="btn btn-primary" disabled={busy} onclick={doExport}>{$t('msg_id_export_confirm')}</button>
        </div>
      {/if}
    </div>
  {:else if panel === 'delete'}
    <div class="panel">
      <p class="warn">{$t('msg_id_delete_warning')}</p>
      {#if error}<div class="error-msg">{error}</div>{/if}
      <div class="actions end">
        <button class="btn btn-ghost" disabled={busy} onclick={() => open('none')}>{$t('msg_back')}</button>
        <button class="btn btn-danger" disabled={busy} onclick={doDelete}>{$t('msg_id_delete_confirm')}</button>
      </div>
    </div>
  {/if}
</div>

<style>
  .identity { max-width: 640px; display: flex; flex-direction: column; gap: var(--sp-3); }
  .card-title { display: flex; align-items: center; gap: var(--sp-2); }
  .meta { color: var(--text-3); font-size: var(--fs-xs); }
  .muted { color: var(--text-2); font-size: var(--fs-sm); margin: 0; }
  .locked-note {
    display: flex; align-items: center; gap: var(--sp-2);
    font-size: var(--fs-sm); color: var(--text-2);
    background: var(--surface-2); border-radius: var(--radius-sm); padding: var(--sp-2) var(--sp-3);
  }
  .actions { display: flex; gap: var(--sp-2); flex-wrap: wrap; }
  .actions.end { justify-content: flex-end; }
  .panel {
    display: flex; flex-direction: column; gap: var(--sp-3);
    border-top: 1px solid var(--border); padding-top: var(--sp-3);
  }
  .warn {
    margin: 0; font-size: var(--fs-sm); color: var(--text-body);
    background: var(--danger-bg); border-radius: var(--radius-sm); padding: var(--sp-2) var(--sp-3);
  }
  .field { display: flex; flex-direction: column; gap: 6px; font-size: var(--fs-sm); }
  .field span { color: var(--text-3); font-size: var(--fs-xs); text-transform: uppercase; letter-spacing: 0.5px; }
  .field input {
    font: inherit; color: var(--text);
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-sm);
    padding: 8px 10px;
  }
  .field input:focus { outline: none; border-color: var(--accent-border); }
  .btn.danger { color: var(--danger-text); }
</style>
