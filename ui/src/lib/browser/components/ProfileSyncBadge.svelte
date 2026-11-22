<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/browser/api';
  import { profileFilesStore } from '$lib/browser/store/profileFiles.svelte';
  import { syncStore } from '$lib/core/store/sync.svelte';
  import { t } from '$lib/core/i18n';
  import { formatError } from '$lib/core/utils';
  import Icon from '$lib/core/Icon.svelte';
  import Notice from '$lib/core/ui/Notice.svelte';

  /** Lease badge, files not sent or not here, and diverged-files actions for one profile. `compact` hides the buttons. */
  let { profileId, compact = false }: { profileId: string; compact?: boolean } = $props();

  onMount(() => {
    const offs: (() => void)[] = [];
    void syncStore.listen().then((fn) => offs.push(fn));
    void profileFilesStore.listen().then((fn) => offs.push(fn));
    return () => offs.forEach((fn) => fn());
  });

  const lease = $derived(syncStore.status?.profile_leases.find((l) => l.profile_id === profileId && !l.own) ?? null);
  const files = $derived(profileFilesStore.byId[profileId] ?? null);
  const unsentLabel = $derived(
    !files ? '' :
    files.state === 'sending' ? $t('profile_sync_files_sending', { done: String(files.done), total: String(files.total) }) :
    files.state === 'open' ? $t('profile_sync_files_open') :
    files.state === 'failed' ? $t('profile_sync_files_failed') : '',
  );
  const unsentHint = $derived(
    files?.state === 'open' ? [$t('profile_sync_files_open_hint'), files.detail].filter(Boolean).join(' — ') : files?.detail ?? '',
  );
  const diverged = $derived(syncStore.status?.profile_conflicts.some((c) => c.note_id === profileId) ?? false);

  let busy = $state(false);
  let error = $state('');

  async function resolve(take: 'remote' | 'mine') {
    busy = true; error = '';
    try {
      syncStore.status = take === 'remote'
        ? await api.sync.profileTakeRemote(profileId)
        : await api.sync.profilePushMine(profileId);
    } catch (e) { error = formatError(e); }
    finally { busy = false; }
  }
</script>

{#if diverged}
  {#if compact}
    <span class="badge badge-danger" title={$t('profile_sync_diverged_hint')}>
      <Icon name="alert-triangle" size={11} /> {$t('profile_sync_diverged')}
    </span>
  {:else}
    <div class="sync-block">
      <span class="badge badge-danger"><Icon name="alert-triangle" size={11} /> {$t('profile_sync_diverged')}</span>
      <span class="hint">{$t('profile_sync_diverged_hint')}</span>
      <div class="sync-actions">
        <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => resolve('remote')}>{$t('profile_sync_take_remote')}</button>
        <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => resolve('mine')}>{$t('profile_sync_push_mine')}</button>
      </div>
      {#if error}<Notice tone="danger">{error}</Notice>{/if}
    </div>
  {/if}
{:else if lease && files?.state === 'elsewhere'}
  <span class="badge badge-warn" class:block={!compact} title={$t('profile_sync_files_elsewhere_hint')}>
    <Icon name="monitor" size={11} /> {$t('profile_sync_files_elsewhere', { device: lease.device_name || lease.device_id })}
  </span>
{:else if lease}
  <span class="badge badge-warn" class:block={!compact} title={$t('profile_sync_in_use_hint')}>
    <Icon name="monitor" size={11} /> {$t('profile_sync_in_use', { device: lease.device_name || lease.device_id })}
  </span>
{:else if unsentLabel}
  <span class="badge" class:badge-warn={files?.state !== 'failed'} class:badge-danger={files?.state === 'failed'} class:block={!compact} title={unsentHint}>
    <Icon name={files?.state === 'sending' ? 'upload' : 'alert-triangle'} size={11} /> {unsentLabel}
  </span>
{/if}

<style>
  /* The state is the common .badge: warn — the files are elsewhere or not sent, danger — diverged or failed. */
  .badge.block { margin-bottom: var(--sp-2); }
  .sync-block { display: flex; flex-direction: column; gap: var(--sp-1-5); align-items: flex-start; margin-bottom: var(--sp-2-5); }
  .sync-actions { display: flex; gap: var(--sp-1-5); }
</style>
