<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Under a password's row in a note's context sheet: the live codes of its TOTP entries. -->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/mobile/i18n';
  import TotpLiveCode from '$lib/pass/components/TotpLiveCode.svelte';
  import { passwordStore } from '$lib/pass/store/passwords.svelte';
  import { totpStore } from '$lib/pass/store/totp.svelte';
  import { totpName } from '$lib/pass/entities';

  let { id }: { id: string } = $props();

  const codes = $derived(passwordStore.list.find((e) => e.id === id)?.totp_ids ?? []);
  const period = (tid: string) => totpStore.list.find((e) => e.id === tid)?.period ?? 30;
</script>

{#if codes.length}
  <div class="pw-codes">
    {#each codes as tid (tid)}
      <div class="pw-code">
        <Icon name="key" size={14} />
        <span class="sub pw-code-name">{totpName(tid)}</span>
        <TotpLiveCode entryId={tid} period={period(tid)} copiedLabel={$t('notes_context_copied')} compact />
      </div>
    {/each}
  </div>
{/if}

<style>
  .sub { font-size: var(--fs-xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pw-codes { display: flex; flex-direction: column; padding: 0 var(--sp-2) var(--sp-1) var(--sp-5); color: var(--text-3); }
  .pw-code { display: flex; align-items: center; gap: var(--sp-2); min-height: 36px; }
  .pw-code-name { flex: 1; min-width: 0; }
</style>
