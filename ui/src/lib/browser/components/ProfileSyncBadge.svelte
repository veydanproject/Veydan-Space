<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/browser/api';
  import { syncStore } from '$lib/core/store/sync.svelte';
  import { t } from '$lib/core/i18n';
  import { formatError } from '$lib/core/utils';
  import Icon from '$lib/core/Icon.svelte';

  /** Lease badge and diverged-files actions for one profile. `compact` hides the buttons. */
  let { profileId, compact = false }: { profileId: string; compact?: boolean } = $props();

  onMount(() => {
    let off: (() => void) | null = null;
    void syncStore.listen().then((fn) => (off = fn));
    return () => off?.();
  });

  const lease = $derived(syncStore.status?.profile_leases.find((l) => l.profile_id === profileId && !l.own) ?? null);
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
    <span class="sync-badge diverged" title={$t('profile_sync_diverged_hint')}>
      <Icon name="alert-triangle" size={11} /> {$t('profile_sync_diverged')}
    </span>
  {:else}
    <div class="sync-block">
      <span class="sync-badge diverged"><Icon name="alert-triangle" size={11} /> {$t('profile_sync_diverged')}</span>
      <span class="hint">{$t('profile_sync_diverged_hint')}</span>
      <div class="sync-actions">
        <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => resolve('remote')}>{$t('profile_sync_take_remote')}</button>
        <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => resolve('mine')}>{$t('profile_sync_push_mine')}</button>
      </div>
      {#if error}<div class="error-msg">{error}</div>{/if}
    </div>
  {/if}
{:else if lease}
  <span class="sync-badge lease" class:block={!compact} title={$t('profile_sync_in_use_hint')}>
    <Icon name="monitor" size={11} /> {$t('profile_sync_in_use', { device: lease.device_name || lease.device_id })}
  </span>
{/if}

<style>
  .sync-badge {
    display: inline-flex; align-items: center; gap: 0.3rem;
    padding: 0.1rem 0.45rem; border-radius: 999px;
    font-size: var(--fs-xs); border: 1px solid var(--border);
    white-space: nowrap;
  }
  .sync-badge.block { margin-bottom: 0.5rem; }
  .sync-badge.lease { background: var(--warn-bg); color: var(--warn-text); border-color: var(--warn-border); }
  .sync-badge.diverged { background: var(--danger-bg); color: var(--danger-text); border-color: var(--danger-border); }
  .sync-block { display: flex; flex-direction: column; gap: 0.35rem; align-items: flex-start; margin-bottom: 0.6rem; }
  .sync-block .hint { font-size: var(--fs-xs); color: var(--text-muted); }
  .sync-actions { display: flex; gap: 0.4rem; }
</style>
