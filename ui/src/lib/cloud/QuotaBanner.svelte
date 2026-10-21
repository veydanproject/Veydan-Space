<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The room of Veydan Cloud runs out (internal/vcloud-plan.md 4.7): at 90 %
  a warning, at 100 % «files do not sync», each with three ways out — more
  room (the page of the cloud: a key of activation), the large files (what
  takes the room, by size) and an own storage (the sync settings, where a
  vault is made anew from this device's data). Shown only on a device that
  syncs through the cloud, and asked only there: another backend of sync
  sends nothing to the project's hosts (the timer stops until a cycle of
  sync finds the cloud again). Closed, it stays closed for the run until
  the level rises.
-->
<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import { api } from '$lib/core/api';
  import { page } from '$app/state';
  import Icon from '$lib/core/Icon.svelte';
  import { isMobile } from '$lib/core/platform';
  import { formatBytes } from '$lib/core/utils';
  import { bannerOf, fullestPool, largest, usageApi, type CloudUsage } from '$lib/cloud/usage';

  const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
  /** How often the numbers are asked again, and the least gap after a cycle of sync. */
  const EVERY_MS = 15 * 60 * 1000;
  const AFTER_SYNC_MS = 60 * 1000;

  let usage = $state<CloudUsage | null>(null);
  let closedAt = $state(0);
  let listOpen = $state(false);
  let askedAt = 0;
  let timer: ReturnType<typeof setInterval> | null = null;
  let unlisten: (() => void) | null = null;

  /** The words of a category of `cloud_usage`. */
  const CATEGORY = {
    journal: 'cloud_quota_cat_journal',
    files: 'cloud_quota_cat_files',
    large_files: 'cloud_quota_cat_large_files',
    recovery: 'cloud_quota_cat_recovery',
    other: 'cloud_quota_cat_other',
  } as const;

  const kind = $derived(bannerOf(usage));
  /** The page of the cloud says the same itself, with the plans beside it: no banner over it. */
  const onCloudPage = $derived(page.url.pathname === '/cloud');
  const shown = $derived(kind !== 'none' && (usage?.level ?? 0) > closedAt && !onCloudPage);
  const pool = $derived(fullestPool(usage));
  const categories = $derived(largest(usage));
  const text = $derived.by(() => {
    if (!pool) return '';
    if (kind === 'warn') return $t('cloud_quota_warn', { percent: String(pool.percent), limit: formatBytes(pool.limit) });
    if (pool.pool === 'total') return $t('cloud_quota_full_total');
    return pool.pool === 'text' ? $t('cloud_quota_full_text') : $t('cloud_quota_full_files');
  });

  /** Whether this device syncs through Veydan Cloud (`sync_backend` = `cloud`). */
  async function onCloud(): Promise<boolean> {
    try {
      return (await api.sync.getConfig())?.backend === 'cloud';
    } catch {
      // A product without sync: nothing to watch.
      return false;
    }
  }

  async function check() {
    askedAt = Date.now();
    if (!(await onCloud())) {
      usage = null;
      if (timer) clearInterval(timer);
      timer = null;
      return;
    }
    timer ??= setInterval(() => void check(), EVERY_MS);
    try {
      usage = await usageApi.usage();
    } catch {
      // No host, no account, no network: nothing to say.
      usage = null;
    }
  }

  function ownStorage() {
    void goto(isMobile ? '/settings/sync' : '/settings#sync');
  }

  onMount(async () => {
    void check();
    if (isTauri) {
      const { listen } = await import('@tauri-apps/api/event');
      unlisten = await listen('sync://status', () => {
        if (Date.now() - askedAt >= AFTER_SYNC_MS) void check();
      });
    }
  });

  onDestroy(() => {
    if (timer) clearInterval(timer);
    unlisten?.();
  });
</script>

{#if shown}
  <div class="quota" class:full={kind === 'full'} role="status">
    <span class="mark"><Icon name="alert-triangle" size={16} /></span>
    <div class="body">
      <div class="head">
        <p class="text">{text}</p>
        <button class="close" type="button" aria-label={$t('cloud_quota_close')} onclick={() => (closedAt = usage?.level ?? 0)}><Icon name="x" size={14} /></button>
      </div>
      <div class="buttons">
        <button class="btn btn-primary btn-sm" type="button" onclick={() => goto('/cloud?section=plans')}>{$t('cloud_quota_more')}</button>
        <button class="btn btn-ghost btn-sm" type="button" aria-expanded={listOpen} onclick={() => (listOpen = !listOpen)}>{$t('cloud_quota_delete')}</button>
        <button class="btn btn-ghost btn-sm" type="button" onclick={ownStorage}>{$t('cloud_quota_own')}</button>
      </div>
      {#if listOpen}
        <div class="list">
          <span class="section-label">{$t('cloud_quota_what')}</span>
          <ul>
            {#each categories as c (c.id)}
              <li><span>{$t(CATEGORY[c.id] ?? CATEGORY.other)}</span><span class="size mono">{formatBytes(c.bytes)}</span></li>
            {/each}
          </ul>
          <p class="hint">{$t('cloud_quota_by_path')}</p>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  /* A card of the column of Banners.svelte, of the same shape as an alert of recovery. */
  .quota {
    display: flex; align-items: flex-start; gap: var(--sp-3);
    padding: var(--sp-3) var(--sp-4); border-radius: var(--radius-md);
    background: var(--surface-drawer); border: 1px solid var(--warn-border); box-shadow: var(--shadow-lg);
    font-size: var(--fs-sm); color: var(--text);
  }
  .mark {
    display: inline-flex; align-items: center; justify-content: center; width: 28px; height: 28px; border-radius: 50%; flex-shrink: 0;
    background: var(--warn-bg); color: var(--warn-text);
  }
  .quota.full { border-color: var(--danger-border); }
  .full .mark { background: var(--danger-bg); color: var(--danger-text); }
  .body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-2); }
  .head { display: flex; align-items: flex-start; gap: var(--sp-2); }
  .text { flex: 1; margin: 0; line-height: 1.45; }
  .close {
    display: inline-flex; align-items: center; justify-content: center; width: 28px; height: 28px; flex-shrink: 0;
    border: 0; border-radius: var(--radius-sm); background: none; color: var(--text-3); cursor: pointer;
  }
  .close:hover { background: var(--surface-hover); color: var(--text); }
  .buttons { display: flex; flex-wrap: wrap; gap: var(--sp-2); }
  .list { display: flex; flex-direction: column; gap: var(--sp-2); padding-top: var(--sp-2); border-top: 1px solid var(--border); }
  ul { margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 4px; }
  li { display: flex; justify-content: space-between; gap: var(--sp-3); color: var(--text-body); }
  .size { color: var(--text-2); white-space: nowrap; }
  .hint { margin: 0; color: var(--text-3); line-height: 1.4; }
</style>
