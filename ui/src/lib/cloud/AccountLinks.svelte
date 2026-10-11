<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- What the owner of the key keeps, on the page «Account»: the profile, the servers, the copy of the key. The rows are the owner's (core/account): this page shows them and knows nothing of where they lead. -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { accountSource, type AccountLink } from '$lib/core/account';
  import { t, type TranslationKey } from '$lib/core/i18n';
  import NavRow from '$lib/cloud/NavRow.svelte';
  import RowList from '$lib/cloud/RowList.svelte';

  let links = $state<AccountLink[]>([]);

  onMount(async () => {
    try {
      links = (await accountSource()?.links()) ?? [];
    } catch {
      links = [];
    }
  });
</script>

{#if links.length > 0}
  <RowList label={$t('cloud_account_owner')}>
    {#each links as link (link.id)}
      <NavRow icon={link.icon} title={$t(link.title as TranslationKey)} sub={link.value ? $t(link.value as TranslationKey) : ''} href={link.href} />
    {/each}
  </RowList>
{/if}
