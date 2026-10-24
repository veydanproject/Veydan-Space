<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The notes' part of the phone's sync settings: the device-local attachment
  download policy. The form's Save calls `save()`.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/mobile/i18n';
  import { api, type NoteAttachmentPolicy } from '$lib/notes/api';

  let policy = $state<NoteAttachmentPolicy | null>(null);

  const clampNum = (raw: string, [lo, hi]: readonly [number, number]) => Math.min(hi, Math.max(lo, Math.floor(Number(raw) || lo)));

  onMount(() => {
    api.notes.attachmentPolicyGet().then((p) => (policy = p)).catch(() => {});
  });

  /** Persists the policy; the form calls it with its own save. */
  export async function save(): Promise<void> {
    if (policy) policy = await api.notes.attachmentPolicySet($state.snapshot(policy));
  }
</script>

<!-- One card: the switch with its words in its row, the threshold it governs under a line. -->
{#if policy}
  <div class="m-section">{$t('settings_att_section')}</div>
  <div class="m-list group">
    <button class="m-row switch" role="switch" aria-checked={policy.download_on_sync} onclick={() => (policy!.download_on_sync = !policy!.download_on_sync)}>
      <span class="words">
        <span class="m-row-label wrap">{$t('settings_att_download_on_sync')}</span>
        <span class="sub">{$t('settings_att_download_on_sync_hint')}</span>
      </span>
      <span class="toggle" class:on={policy.download_on_sync}></span>
    </button>
    <div class="m-field ask">
      <label for="att-ask">{$t('settings_att_ask_above')}</label>
      <input id="att-ask" type="number" inputmode="numeric" min="1" max="4096"
        value={policy.ask_above_mib}
        oninput={(e) => (policy!.ask_above_mib = clampNum((e.currentTarget as HTMLInputElement).value, [1, 4096]))} />
      <span class="sub">{$t('settings_att_ask_above_hint')}</span>
    </div>
  </div>
{/if}

<style>
  .group { margin-bottom: var(--sp-4); }
  /* A sentence, not a name: it wraps instead of being cut. */
  .wrap { white-space: normal; overflow: visible; line-height: 1.3; }
  .switch { min-height: 64px; }
  .words { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .sub { font-size: var(--fs-sm); line-height: 1.35; color: var(--text-2); }
  .ask { margin: 0; padding: var(--sp-3) var(--sp-4) var(--sp-4); border-top: 1px solid var(--divider); }
</style>
