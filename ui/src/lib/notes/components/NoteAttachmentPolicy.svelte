<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { api, DEFAULT_ATTACHMENT_POLICY, type NoteAttachmentPolicy } from '$lib/notes/api';
  import { t } from '$lib/core/i18n';
  import { formatError } from '$lib/core/utils';

  let policy = $state<NoteAttachmentPolicy>({ ...DEFAULT_ATTACHMENT_POLICY });
  let loaded = $state(false);
  let saved = $state(false);
  let error = $state('');

  onMount(async () => {
    try {
      policy = await api.notes.attachmentPolicyGet();
    } catch {}
    loaded = true;
  });

  const num = (e: Event, lo: number) => Math.max(lo, Math.floor(Number((e.currentTarget as HTMLInputElement).value) || lo));

  async function save() {
    error = '';
    try {
      policy = await api.notes.attachmentPolicySet($state.snapshot(policy));
      saved = true;
      setTimeout(() => (saved = false), 2000);
    } catch (e) {
      error = formatError(e);
    }
  }
</script>

{#if loaded}
  <div class="policy">
    <div class="toggle-row">
      <div class="toggle-info">
        <span>{$t('settings_att_large_enabled')}</span>
        <span class="hint">{$t('settings_att_large_enabled_hint')}</span>
      </div>
      <button class="toggle" class:on={policy.large_files_enabled} onclick={() => (policy.large_files_enabled = !policy.large_files_enabled)} aria-pressed={policy.large_files_enabled} aria-label={$t('settings_att_large_enabled')}></button>
    </div>
    <div class="grid">
      <label class="field">
        <span class="field-label">{$t('settings_att_threshold')}</span>
        <input class="input" type="number" min="1" value={policy.threshold_mib} disabled={!policy.large_files_enabled}
          oninput={(e) => (policy.threshold_mib = num(e, 1))} />
        <span class="hint">{$t('settings_att_threshold_hint')}</span>
      </label>
      <label class="field">
        <span class="field-label">{$t('settings_att_max')}</span>
        <input class="input" type="number" min="0" value={policy.max_file_gib}
          oninput={(e) => (policy.max_file_gib = num(e, 0))} />
        <span class="hint">{$t('settings_att_max_hint')}</span>
      </label>
    </div>
    <div class="toggle-row">
      <div class="toggle-info">
        <span>{$t('settings_att_download_on_sync')}</span>
        <span class="hint">{$t('settings_att_download_on_sync_hint')}</span>
      </div>
      <button class="toggle" class:on={policy.download_on_sync} onclick={() => (policy.download_on_sync = !policy.download_on_sync)} aria-pressed={policy.download_on_sync} aria-label={$t('settings_att_download_on_sync')}></button>
    </div>
    <label class="field">
      <span class="field-label">{$t('settings_att_ask_above')}</span>
      <input class="input" type="number" min="1" value={policy.ask_above_mib}
        oninput={(e) => (policy.ask_above_mib = num(e, 1))} />
      <span class="hint">{$t('settings_att_ask_above_hint')}</span>
    </label>
    <p class="hint">{$t('settings_att_compat_warn')}</p>
    <div class="row">
      <button class="btn btn-primary btn-sm" onclick={save}>{$t('settings_notes_save')}</button>
      {#if saved}<span class="ok">✓</span>{/if}
    </div>
    {#if error}<p class="error">{error}</p>{/if}
  </div>
{/if}

<style>
  .policy { display: flex; flex-direction: column; gap: var(--sp-3); }
  .row { display: flex; gap: var(--sp-2); align-items: center; }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: var(--sp-3); }
  .field { display: flex; flex-direction: column; gap: var(--sp-2); min-width: 0; }
  .field-label { font-size: var(--fs-sm); color: var(--text); font-weight: var(--fw-semibold); }
  .input {
    width: 100%; min-width: 0; height: var(--control-h-lg);
    background: var(--surface-3); border: 1px solid var(--border);
    border-radius: var(--radius-field); padding: 0 var(--sp-3);
    font-size: 0.82rem; color: var(--text-body);
  }
  .input:focus { border-color: var(--accent-border); box-shadow: 0 0 0 3px var(--accent-bg); outline: none; }
  .input:disabled { opacity: 0.5; }
  .hint { font-size: var(--fs-sm); color: var(--text-dim); margin: 0; }
  .ok { font-size: var(--fs-sm); color: var(--success-text); margin: 0; }
  .error { font-size: var(--fs-sm); color: var(--danger-text); margin: 0; }
  .toggle-row { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-4); }
  .toggle-info { display: flex; flex-direction: column; gap: 0.2rem; font-size: var(--fs-base); }
</style>
