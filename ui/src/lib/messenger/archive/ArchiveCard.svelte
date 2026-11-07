<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- «Архив переписки» in the settings of the chats (messenger-archive-plan.md section 5): what the archive holds, for what period, and what it does now. No buttons in this version. -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t, locale } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { bytes } from '../shared/format';
  import { archiveStore } from './archiveStore.svelte';
  import { dateOf, monthLabel, stateOf } from './archive';

  // The size and the months are in the status alone: read anew each time the card is shown.
  onMount(() => { archiveStore.load().catch(() => {}); });

  const status = $derived(archiveStore.status);
  const state = $derived(stateOf(status?.available ?? false, archiveStore.progress, status?.paused ?? null));
  const reason = $derived(status?.paused ?? archiveStore.progress.reason ?? null);
  const until = $derived(reason?.why === 'read_only' && reason.until ? dateOf(reason.until, $locale) : '');

  const from = $derived(monthLabel(status?.from_month, $locale));
  const to = $derived(monthLabel(status?.to_month, $locale));
  const period = $derived(from && to ? $t('msg_archive_range', { from, to }) : to ? $t('msg_archive_until', { to }) : $t('msg_archive_empty'));

  const line = $derived(
    {
      writing: () => $t('msg_archive_st_writing'),
      idle: () => $t('msg_archive_st_idle'),
      restoring: () => $t('msg_archive_st_restoring', { percent: String(archiveStore.percent) }),
      paused_read_only: () => $t('msg_archive_st_read_only'),
      paused_read_only_until: () => $t('msg_archive_st_read_only_until', { date: until }),
      paused_quota: () => $t('msg_archive_st_quota'),
      paused_offline: () => $t('msg_archive_st_offline'),
      unavailable: () => $t('msg_archive_st_unavailable'),
    }[state](),
  );
  /** Months whose files are too large to restore here: they stay in the storage, the card names them. */
  const tooLarge = $derived((status?.too_large ?? []).map((m) => monthLabel(m, $locale)).filter(Boolean).join(', '));
  /** Months of this device too large to write: in no backup, the card names them. */
  const notBackedUp = $derived((status?.not_backed_up ?? []).map((m) => monthLabel(m, $locale)).filter(Boolean).join(', '));
  /** A stopped archive is told in the warning tone; a restore and a write are the quiet ones. */
  const stopped = $derived(state.startsWith('paused'));
</script>

{#if status}
  <section class="card archive" aria-labelledby="archive-title">
    <div class="head">
      <span class="tile"><Icon name="archive" size={20} /></span>
      <div class="card-title" id="archive-title">{$t('msg_archive_title')}</div>
    </div>
    {#if state === 'unavailable'}
      <p class="state off">{line}</p>
    {:else}
      <dl class="grid">
        <dt>{$t('msg_archive_size')}</dt><dd>{bytes(status.size_bytes, $locale)}</dd>
        <dt>{$t('msg_archive_period')}</dt><dd>{period}</dd>
        <dt>{$t('msg_archive_state')}</dt><dd class="state" class:stopped>{line}</dd>
      </dl>
      {#if notBackedUp}
        <p class="state stopped">{$t('msg_archive_not_backed_up', { months: notBackedUp })}</p>
      {/if}
      {#if tooLarge}
        <p class="state stopped">{$t('msg_archive_too_large', { months: tooLarge })}</p>
      {/if}
      <p class="hint">{$t('msg_archive_hint')}</p>
    {/if}
  </section>
{/if}

<style>
  .archive { display: flex; flex-direction: column; gap: var(--sp-3); }
  .head { display: flex; align-items: center; gap: var(--sp-3); }
  .tile {
    flex-shrink: 0; display: inline-flex; align-items: center; justify-content: center; width: 36px; height: 36px;
    border-radius: var(--radius-sm); background: var(--accent-bg); color: var(--accent-text);
  }
  .grid { display: grid; grid-template-columns: max-content 1fr; gap: var(--sp-2) var(--sp-4); margin: 0; font-size: var(--fs-sm); }
  dt { color: var(--text-3); }
  dd { margin: 0; min-width: 0; color: var(--text-body); }
  .state { margin: 0; font-size: var(--fs-sm); color: var(--text-body); line-height: var(--lh-snug); }
  .state.off { color: var(--text-2); }
  .state.stopped { color: var(--warn-text); }
  .hint { margin: 0; }
</style>
