<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The pass module's part of a note's context row for a TOTP entry: its tags, the live code, the open button in .foot. -->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';
  import TotpLiveCode from '$lib/pass/components/TotpLiveCode.svelte';
  import PassTags from '$lib/pass/desktop/PassTags.svelte';
  import { totpStore } from '$lib/pass/store/totp.svelte';
  import { passEntities } from '$lib/pass/entities';

  let { id, children }: { id: string; children?: Snippet } = $props();

  const def = passEntities.find((d) => d.kind === 'totp')!;
  const entry = $derived(totpStore.list.find((e) => e.id === id));
</script>

{@render children?.()}
{#if entry}
  <PassTags tags={entry.tags} />
  <TotpLiveCode entryId={id} period={entry.period ?? 30} copiedLabel={$t('totp_copy')} />
{/if}
<div class="foot">
  <div class="pw-actions">
    <button type="button" class="icon-btn" title={$t('common_open')} onclick={() => def.open?.(id)}>
      <Icon name="external-link" size={18} />
    </button>
  </div>
</div>

<style>
  .foot {
    display: flex;
    align-items: center;
    gap: var(--sp-1-5);
    width: 100%;
  }
  .pw-actions { display: flex; gap: var(--sp-1); margin-left: auto; }
</style>
