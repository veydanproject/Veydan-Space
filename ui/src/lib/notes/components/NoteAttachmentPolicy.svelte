<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Two cards, each a switch row and the numbers it governs; the warning about old versions; the save at the foot. -->
<script lang="ts">
  import { onMount } from 'svelte';
  import ToggleRow from '$lib/core/ui/ToggleRow.svelte';
  import Notice from '$lib/core/ui/Notice.svelte';
  import { api, DEFAULT_ATTACHMENT_POLICY, type NoteAttachmentPolicy } from '$lib/notes/api';
  import { t } from '$lib/core/i18n';
  import { formatError } from '$lib/core/utils';
  import { toast } from '$lib/core/ui/toast.svelte';

  let policy = $state<NoteAttachmentPolicy>({ ...DEFAULT_ATTACHMENT_POLICY });
  let loaded = $state(false);
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
      toast.success($t('common_saved'));
    } catch (e) {
      error = formatError(e);
    }
  }
</script>

{#snippet switchRow(on: boolean, title: string, hint: string, flip: () => void)}
  <div class="switch"><ToggleRow surface="row" {title} {hint} {on} onclick={flip} /></div>
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
    <Notice tone="info">{$t('settings_att_compat_warn')}</Notice>
    {#if error}<Notice tone="danger">{error}</Notice>{/if}
    <div class="foot">
      <button type="button" class="btn btn-primary" onclick={save}>{$t('common_save')}</button>
    </div>
  </div>
{/if}

<style>
  .policy { display: flex; flex-direction: column; gap: var(--sp-4); container-type: inline-size; }
  .card.group { padding: var(--sp-1); }
  /* The card's switch (ToggleRow `row`, reaching past this padding by --sp-3). */
  .switch { padding: 0 var(--sp-3); }
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
  .input:focus { border-color: var(--accent-border); outline: none; }
  .hint { line-height: var(--lh-snug); margin: 0; }
  .foot { display: flex; justify-content: flex-end; align-items: center; gap: var(--sp-3); }

  /* A narrow window: the numbers one under another. */
  @container (max-width: 560px) {
    .nums { grid-template-columns: minmax(0, 1fr); }
  }
</style>
