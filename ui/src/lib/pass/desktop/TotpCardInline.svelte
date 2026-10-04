<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The pass module's part of a note's context card for a TOTP entry: its tags, the live code, the open button. -->
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

{#if entry}
  <PassTags tags={entry.tags} />
  <TotpLiveCode entryId={id} period={entry.period ?? 30} copiedLabel={$t('totp_copy')} />
{/if}
{@render children?.()}
<div class="foot">
  <div class="pw-actions">
    <button type="button" class="icon-btn" title={$t('ctx_action_open')} onclick={() => def.open?.(id)}>
      <Icon name="external-link" size={13} />
    </button>
  </div>
</div>

<style>
  .foot {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    width: 100%;
    margin-top: auto;
  }
  .pw-actions { display: flex; gap: 0.25rem; margin-left: auto; }
</style>
