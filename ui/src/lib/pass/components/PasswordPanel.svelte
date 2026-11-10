<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import { appLock } from '$lib/core/lock/store.svelte';
  import { passwordStore } from '$lib/pass/store/passwords.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import PasswordList from './PasswordList.svelte';
  import PasswordCard from './PasswordCard.svelte';
  import PasswordEditModal from './PasswordEditModal.svelte';
  import PasswordLockBadge from './PasswordLockBadge.svelte';

  interface Props {
    profileId: string;
  }

  let { profileId }: Props = $props();

  let selectedId = $state<string | null>(null);
  let creating = $state(false);
  let editingId = $state<string | null>(null);

  const entries = $derived(passwordStore.byProfile(profileId));
  const selected = $derived(entries.find((entry) => entry.id === selectedId) ?? null);
  const editingEntry = $derived(passwordStore.list.find((entry) => entry.id === editingId) ?? null);
  const canWrite = $derived(!appLock.locked && appLock.status.vault !== 'mismatch');
  const formOpen = $derived(creating || !!editingEntry);

  /** «Отмена» of the form: back to the password it was opened from, else to the list. */
  function closeForm() {
    creating = false;
    editingId = null;
  }

  onMount(() => {
    void appLock.refresh();
    void passwordStore.ensureLoaded();
  });
</script>

<!-- A password and its form are shown in the tab in place of the list, their buttons at its end. -->
<div class="panel">
  <div class="bar">
    <span class="count"><PasswordLockBadge size={9} />{entries.length}</span>
    {#if canWrite && !formOpen}
      <button class="icon-btn create" onclick={() => (creating = true)} title={$t('common_add')} aria-label={$t('common_add')}>
        <Icon name="plus" size={18} />
      </button>
    {/if}
  </div>
  {#if formOpen || selected}
    <button type="button" class="btn btn-ghost btn-sm" onclick={() => { selectedId = null; closeForm(); }}>{$t('common_back')}</button>
  {/if}
  {#if formOpen}
    <PasswordEditModal
      entry={creating ? null : editingEntry}
      initialTags={creating ? [`profile:${profileId}`] : []}
      buttons="end"
      onclose={closeForm}
    />
  {:else if selected}
    <PasswordCard entry={selected} onedit={() => (editingId = selected.id)} ondeleted={() => (selectedId = null)} />
  {:else if entries.length === 0}
    <p class="muted">{$t('pw_empty_profile')}</p>
  {:else}
    <PasswordList entries={entries} onopen={(id) => (selectedId = id)} onedit={(entry) => (editingId = entry.id)} />
  {/if}
</div>

<style>
  .panel { display: flex; flex-direction: column; gap: var(--sp-3); }
  .bar { display: flex; justify-content: space-between; align-items: center; color: var(--text-faint); font-size: var(--fs-sm); }
  .count { display: inline-flex; align-items: center; gap: 2px; }
  .muted { color: var(--text-2); font-size: var(--fs-base); }
</style>
