<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Two cards, each a switch row and the numbers it governs; the warning about old versions; the save at the foot. -->
<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
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

{#snippet switchRow(on: boolean, title: string, hint: string, flip: () => void)}
  <button type="button" class="switch-row" role="switch" aria-checked={on} onclick={flip}>
    <span class="words">
      <span class="title">{title}</span>
      <span class="hint">{hint}</span>
    </span>
    <span class="toggle" class:on aria-hidden="true"></span>
  </button>
{/snippet}

{#if loaded}
  <div class="policy">
    <div class="card group">
      {@render switchRow(policy.large_files_enabled, $t('settings_att_large_enabled'), $t('settings_att_large_enabled_hint'), () => (policy.large_files_enabled = !policy.large_files_enabled))}
      <div class="nums">
        <label class="field" class:off={!policy.large_files_enabled}>
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
    </div>
    <div class="card group">
      {@render switchRow(policy.download_on_sync, $t('settings_att_download_on_sync'), $t('settings_att_download_on_sync_hint'), () => (policy.download_on_sync = !policy.download_on_sync))}
      <div class="nums">
        <label class="field">
          <span class="field-label">{$t('settings_att_ask_above')}</span>
          <input class="input" type="number" min="1" value={policy.ask_above_mib}
            oninput={(e) => (policy.ask_above_mib = num(e, 1))} />
          <span class="hint">{$t('settings_att_ask_above_hint')}</span>
        </label>
      </div>
    </div>
    <p class="note"><Icon name="info" size={18} /><span>{$t('settings_att_compat_warn')}</span></p>
    {#if error}
      <div class="fail" role="alert"><Icon name="alert-triangle" size={18} /><span>{error}</span></div>
    {/if}
    <div class="foot">
      {#if saved}<span class="ok" role="status">{$t('note_status_saved')}</span>{/if}
      <button type="button" class="btn btn-primary" onclick={save}>{$t('settings_notes_save')}</button>
    </div>
  </div>
{/if}

<style>
  .policy { display: flex; flex-direction: column; gap: var(--sp-4); container-type: inline-size; }
  .card.group { padding: var(--sp-1); }
  .switch-row {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; min-height: 64px; padding: var(--sp-2) var(--sp-4);
    border: 0; border-radius: 0; background: none; color: var(--text); font: inherit; text-align: left; cursor: pointer;
    transition: background var(--dur-fast);
  }
  /* The hover of a row: square between lines, the first and the last row take the card's corners (its radius less its padding). */
  .switch-row:first-child { border-top-left-radius: calc(var(--radius-card) - var(--sp-1)); border-top-right-radius: calc(var(--radius-card) - var(--sp-1)); }
  .switch-row:last-child { border-bottom-left-radius: calc(var(--radius-card) - var(--sp-1)); border-bottom-right-radius: calc(var(--radius-card) - var(--sp-1)); }
  .switch-row:hover { background: var(--surface-row-hover); }
  .switch-row .toggle { pointer-events: none; }
  .words { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .title { font-size: var(--fs-body); font-weight: var(--fw-semibold); }
  .nums {
    display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--sp-4);
    padding: var(--sp-3) var(--sp-4) var(--sp-4); border-top: 1px solid var(--divider);
  }
  .field { display: flex; flex-direction: column; gap: var(--sp-2); min-width: 0; }
  .field.off { opacity: 0.5; }
  .field-label { font-size: var(--fs-sm); color: var(--text); font-weight: var(--fw-semibold); }
  .input {
    width: 100%; min-width: 0; height: var(--control-h-lg);
    background: var(--surface-3); border: 1px solid var(--border);
    border-radius: var(--radius-field); padding: 0 var(--sp-3);
    font-size: var(--fs-sm); color: var(--text-body);
  }
  .input:focus { border-color: var(--accent-border); box-shadow: 0 0 0 3px var(--accent-bg); outline: none; }
  .hint { font-size: var(--fs-sm); line-height: 1.4; color: var(--text-2); margin: 0; }
  .note { display: flex; align-items: flex-start; gap: var(--sp-3); margin: 0; font-size: var(--fs-sm); color: var(--text-2); }
  .note :global(svg) { flex-shrink: 0; color: var(--text-dim); }
  .fail {
    display: flex; align-items: flex-start; gap: var(--sp-3); padding: var(--sp-3) var(--sp-4);
    border-radius: var(--radius-md); background: var(--danger-bg); color: var(--text); font-size: var(--fs-base); line-height: 1.5;
    overflow-wrap: anywhere;
  }
  .fail :global(svg) { flex-shrink: 0; margin-top: 2px; color: var(--danger-text); }
  .foot { display: flex; justify-content: flex-end; align-items: center; gap: var(--sp-3); }
  .ok { font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--success-text); }

  /* A narrow window: the numbers one under another. */
  @container (max-width: 560px) {
    .nums { grid-template-columns: minmax(0, 1fr); }
  }
</style>
