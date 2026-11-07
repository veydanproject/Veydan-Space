<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import Notice from '$lib/core/ui/Notice.svelte';
  import { t } from '$lib/core/i18n';
  import ToggleRow from '$lib/core/ui/ToggleRow.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import type { BlockedView, DmFrom } from '../api';
  import InvitesCard from '../invites/InvitesCard.svelte';
  import { privacyStore } from './privacyStore.svelte';

  const DM_FROM: readonly DmFrom[] = ['anyone', 'contacts'];
  /** Days an unanswered request is kept; 0 keeps it until answered. */
  const KEEP_DAYS: readonly number[] = [3, 7, 30, 0];

  onMount(() => {
    privacyStore.load();
    privacyStore.loadDm();
  });

  const view = $derived(privacyStore.view);
  const dm = $derived(privacyStore.dm);
  const dmFrom = $derived<DmFrom>(dm?.dm_from === 'contacts' ? 'contacts' : 'anyone');
  /** The runtime names them by nickname, profile name or a short npub (`npub1…xxxx`): the key line goes under a real name only. */
  const namedByKey = (b: BlockedView) => !b.npub || b.name.startsWith(b.npub.slice(0, 12));
</script>

<div class="card privacy">
  <div class="card-title">{$t('msg_privacy_title')}</div>

  {#if !view}
    <p class="muted">{privacyStore.error || $t('loading')}</p>
  {:else}
    <div class="block">
      <ToggleRow surface="row" title={$t('msg_privacy_read_receipts')} hint={$t('msg_privacy_read_receipts_hint')} label={$t('msg_privacy_read_receipts')}
        on={view.read_receipts} disabled={privacyStore.busy} onclick={() => privacyStore.setReadReceipts(!view.read_receipts)} />
    </div>
    <div class="block">
      <ToggleRow surface="row" title={$t('msg_privacy_presence')} hint={$t('msg_privacy_presence_hint')} label={$t('msg_privacy_presence')}
        on={view.presence} disabled={privacyStore.busy} onclick={() => privacyStore.setPresence(!view.presence)} />
    </div>
    {#if privacyStore.error}<Notice tone="danger">{privacyStore.error}</Notice>{/if}
  {/if}
</div>

<div class="card privacy">
  <div class="card-title">{$t('msg_privacy_dm_title')}</div>

  {#if !dm}
    <p class="muted">{privacyStore.dmError || $t('loading')}</p>
  {:else}
    <div class="block">
      <div class="label">{$t('msg_privacy_dm_from')}</div>
      <div class="seg seg-fill" role="radiogroup" aria-label={$t('msg_privacy_dm_from')}>
        {#each DM_FROM as v (v)}
          <button class="seg-btn" class:active={dmFrom === v} role="radio" aria-checked={dmFrom === v}
            disabled={privacyStore.dmBusy} onclick={() => privacyStore.setDmFrom(v)}>
            {$t(`msg_privacy_dm_from_${v}` as 'msg_privacy_dm_from_anyone')}
          </button>
        {/each}
      </div>
      <p class="muted small">{$t(`msg_privacy_dm_from_${dmFrom}_hint` as 'msg_privacy_dm_from_anyone_hint')}</p>
    </div>
    <div class="block">
      <div class="label">{$t('msg_privacy_keep')}</div>
      <div class="seg seg-fill" role="radiogroup" aria-label={$t('msg_privacy_keep')}>
        {#each KEEP_DAYS as d (d)}
          <button class="seg-btn" class:active={dm.keep_days === d} role="radio" aria-checked={dm.keep_days === d}
            disabled={privacyStore.dmBusy} onclick={() => privacyStore.setKeepDays(d)}>
            {$t(`msg_privacy_keep_${d}` as 'msg_privacy_keep_7')}
          </button>
        {/each}
      </div>
      <p class="muted small">{$t('msg_privacy_keep_hint')}</p>
    </div>
    {#if privacyStore.dmError}<Notice tone="danger">{privacyStore.dmError}</Notice>{/if}
  {/if}
</div>

<div class="card privacy">
  <div class="card-title">{$t('msg_privacy_blocked')}</div>

  {#if !privacyStore.blocked}
    <p class="muted">{privacyStore.dmError || $t('loading')}</p>
  {:else if privacyStore.blocked.length === 0}
    <p class="muted">{$t('msg_privacy_blocked_empty')}</p>
  {:else}
    <ul class="blocked">
      {#each privacyStore.blocked as b (b.peer)}
        <li class="line">
          <!-- Initials only: a picture of a blocked person is not fetched. -->
          <span class="who">
            <Avatar url={null} label={b.name} seed={b.peer} size={36} />
            <span class="who-text"><span class="name">{b.name}</span>{#if !namedByKey(b)}<code class="npub">{b.npub.slice(0, 16)}…</code>{/if}</span>
          </span>
          {#if b.banned}
            <span class="muted small">{$t('msg_rel_banned_short')}</span>
          {:else}
            <button class="btn btn-ghost" disabled={privacyStore.unblocking === b.peer} onclick={() => privacyStore.unblock(b.peer)}>
              {$t('msg_rel_cta_unblock')}
            </button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
  {#if privacyStore.blockedError}<Notice tone="danger">{privacyStore.blockedError}</Notice>{/if}
</div>

<InvitesCard />

<style>
  .privacy { display: flex; flex-direction: column; gap: var(--sp-3); }
  .muted { color: var(--text-2); font-size: var(--fs-sm); margin: 0; }
  .small { font-size: var(--fs-sm); }
  .line { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-3); min-height: 40px; font-size: var(--fs-base); }
  /* The settings of a card one under another, a line between them (as «Профиль и ключ»). */
  .block { display: flex; flex-direction: column; gap: var(--sp-2); }
  .block + .block { padding-top: var(--sp-3); border-top: 1px solid var(--divider); }
  .label { font-size: var(--fs-sm); font-weight: var(--fw-bold); color: var(--text); }
  /* Rows of 48 with a line between them (as the facts of «Профиль и ключ»). */
  .blocked { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .blocked .line { min-height: 48px; padding: var(--sp-1) 0; }
  .blocked .line + .line { border-top: 1px solid var(--border); }
  .blocked .btn { flex-shrink: 0; }
  .who { display: flex; align-items: center; gap: var(--sp-3); min-width: 0; }
  .who-text { display: flex; flex-direction: column; min-width: 0; }
  .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .npub { font-size: var(--fs-xs); color: var(--text-3); }
</style>
