<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import { totpStore } from '$lib/pass/store/totp.svelte';
  import TotpList from './TotpList.svelte';
  import TotpAddModal from './TotpAddModal.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import Skeleton from '$lib/core/ui/Skeleton.svelte';
  import type { TotpEntry } from '$lib/pass/types';

  interface Props {
    profileId: string;
  }

  let { profileId }: Props = $props();

  let showAdd = $state(false);
  let editing = $state<TotpEntry | null>(null);
  const formOpen = $derived(showAdd || editing !== null);

  function closeForm() {
    showAdd = false;
    editing = null;
  }

  const profileTag = $derived(`profile:${profileId}`);
  const entries = $derived(totpStore.byProfile(profileId));

  onMount(() => {
    totpStore.ensureLoaded();
  });
</script>

<!-- The form is shown in the tab in place of the list, its buttons at its end. -->
<div class="totp-panel">
  <div class="panel-toolbar">
    <span class="toolbar-count">{entries.length} TOTP</span>
    {#if !formOpen}
      <button class="icon-btn create" onclick={() => (showAdd = true)} title={$t('totp_btn_add')} aria-label={$t('totp_btn_add')}>
        <Icon name="plus" size={18} />
      </button>
    {/if}
  </div>

  {#if formOpen}
    <button type="button" class="btn btn-ghost btn-sm" onclick={closeForm}>{$t('common_back')}</button>
    <TotpAddModal frame="end" entry={editing} initialTags={editing ? [] : [profileTag]} onclose={closeForm} />
  {:else if totpStore.loading && !totpStore.loaded}
    <Skeleton rows={4} height={64} avatar="square" trailing />
  {:else}
    <TotpList
      {entries}
      emptyText={$t('totp_empty_profile')}
      onrequestAdd={() => (showAdd = true)}
      onedit={(entry) => (editing = entry)}
    />
  {/if}
</div>

<style>
  .totp-panel {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }

  .panel-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .toolbar-count {
    font-size: var(--fs-sm);
    color: var(--text-faint);
  }

  /* .btn / .btn-ghost / .btn-sm are global primitives (base.css) */
</style>
