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

{#if policy}
  <div class="m-section">{$t('settings_att_section')}</div>
  <div class="m-list group">
    <button class="m-row" onclick={() => (policy!.download_on_sync = !policy!.download_on_sync)}>
      <span class="m-row-label wrap">{$t('settings_att_download_on_sync')}</span>
      <span class="toggle" class:on={policy.download_on_sync}></span>
    </button>
  </div>
  <p class="m-hint">{$t('settings_att_download_on_sync_hint')}</p>
  <div class="m-field">
    <label for="att-ask">{$t('settings_att_ask_above')}</label>
    <input id="att-ask" type="number" inputmode="numeric" min="1" max="4096"
      value={policy.ask_above_mib}
      oninput={(e) => (policy!.ask_above_mib = clampNum((e.currentTarget as HTMLInputElement).value, [1, 4096]))} />
  </div>
  <p class="m-hint">{$t('settings_att_ask_above_hint')}</p>
{/if}

<style>
  .group { margin-bottom: var(--sp-4); }
  /* A sentence, not a name: it wraps instead of being cut. */
  .wrap { white-space: normal; overflow: visible; line-height: 1.3; }
</style>
