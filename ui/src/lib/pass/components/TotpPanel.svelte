<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import { totpStore } from '$lib/pass/store/totp.svelte';
  import TotpList from './TotpList.svelte';
  import TotpAddModal from './TotpAddModal.svelte';
  import Icon from '$lib/core/Icon.svelte';

  interface Props {
    profileId: string;
  }

  let { profileId }: Props = $props();

  let showAdd = $state(false);

  const profileTag = $derived(`profile:${profileId}`);
  const entries = $derived(totpStore.byProfile(profileId));

  onMount(() => {
    totpStore.ensureLoaded();
  });
</script>

<div class="totp-panel">
  <div class="panel-toolbar">
    <span class="toolbar-count">{entries.length} TOTP</span>
    <button class="btn btn-primary toolbar-add" onclick={() => (showAdd = true)}>
      <Icon name="plus" size={14} /> {$t('totp_btn_add')}
    </button>
  </div>

  {#if totpStore.loading && !totpStore.loaded}
    <div class="loading">{$t('loading')}</div>
  {:else}
    <TotpList
      {entries}
      emptyText={$t('totp_empty_profile')}
      onrequestAdd={() => (showAdd = true)}
    />
  {/if}
</div>

{#if showAdd}
  <TotpAddModal
    initialTags={[profileTag]}
    onclose={() => (showAdd = false)}
  />
{/if}

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
    font-size: 0.78rem;
    color: var(--text-faint);
  }

  .toolbar-add {
    height: 34px;
    padding: 0 13px;
    font-size: 0.82rem;
    border-radius: 9px;
  }

  .loading {
    font-size: var(--fs-sm);
    color: var(--text-2);
    padding: var(--sp-4) 0;
    text-align: center;
  }

  /* .btn / .btn-ghost / .btn-sm are global primitives (base.css) */
</style>
