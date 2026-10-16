<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
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
  <div class="card-title"><Icon name="eye-off" size={16} /> {$t('msg_privacy_title')}</div>

  {#if !view}
    <p class="muted">{privacyStore.error || $t('loading')}</p>
  {:else}
    <div class="block">
      <div class="line">
        <span>{$t('msg_privacy_read_receipts')}</span>
        <button
          class="toggle"
          class:on={view.read_receipts}
          disabled={privacyStore.busy}
          onclick={() => privacyStore.setReadReceipts(!view.read_receipts)}
          aria-pressed={view.read_receipts}
          aria-label={$t('msg_privacy_read_receipts')}
        ></button>
      </div>
      <p class="muted small">{$t('msg_privacy_read_receipts_hint')}</p>
    </div>
    <div class="block">
      <div class="line">
        <span>{$t('msg_privacy_presence')}</span>
        <button
          class="toggle"
          class:on={view.presence}
          disabled={privacyStore.busy}
          onclick={() => privacyStore.setPresence(!view.presence)}
          aria-pressed={view.presence}
          aria-label={$t('msg_privacy_presence')}
        ></button>
      </div>
      <p class="muted small">{$t('msg_privacy_presence_hint')}</p>
    </div>
    {#if privacyStore.error}<p class="warn">{privacyStore.error}</p>{/if}
  {/if}
</div>

<div class="card privacy">
  <div class="card-title"><Icon name="shield" size={16} /> {$t('msg_privacy_dm_title')}</div>

  {#if !dm}
    <p class="muted">{privacyStore.dmError || $t('loading')}</p>
  {:else}
    <div class="block">
      <div class="label">{$t('msg_privacy_dm_from')}</div>
      <div class="seg" role="radiogroup" aria-label={$t('msg_privacy_dm_from')}>
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
      <div class="seg" role="radiogroup" aria-label={$t('msg_privacy_keep')}>
        {#each KEEP_DAYS as d (d)}
          <button class="seg-btn" class:active={dm.keep_days === d} role="radio" aria-checked={dm.keep_days === d}
            disabled={privacyStore.dmBusy} onclick={() => privacyStore.setKeepDays(d)}>
            {$t(`msg_privacy_keep_${d}` as 'msg_privacy_keep_7')}
          </button>
        {/each}
      </div>
      <p class="muted small">{$t('msg_privacy_keep_hint')}</p>
    </div>
    {#if privacyStore.dmError}<p class="warn">{privacyStore.dmError}</p>{/if}
  {/if}
</div>

<div class="card privacy">
  <div class="card-title"><Icon name="ban" size={16} /> {$t('msg_privacy_blocked')}</div>

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
            <Avatar url={null} label={b.name} seed={b.peer} size={32} />
            <span class="who-text"><span class="name">{b.name}</span>{#if !namedByKey(b)}<code class="npub">{b.npub.slice(0, 16)}…</code>{/if}</span>
          </span>
          {#if b.banned}
            <span class="muted small">{$t('msg_rel_banned_short')}</span>
          {:else}
            <button class="btn btn-ghost btn-sm" disabled={privacyStore.unblocking === b.peer} onclick={() => privacyStore.unblock(b.peer)}>
              {$t('msg_rel_cta_unblock')}
            </button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
  {#if privacyStore.blockedError}<p class="warn">{privacyStore.blockedError}</p>{/if}
</div>

<InvitesCard />

<style>
  .privacy { display: flex; flex-direction: column; gap: var(--sp-3); }
  .card-title { display: flex; align-items: center; gap: var(--sp-2); }
  .muted { color: var(--text-2); font-size: var(--fs-sm); margin: 0; }
  .small { font-size: var(--fs-xs); }
  .warn { margin: 0; font-size: var(--fs-sm); color: var(--danger-text); }
  .line { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-3); font-size: var(--fs-sm); }
  .block { display: flex; flex-direction: column; gap: var(--sp-2); }
  .label { font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text); }
  /* A phone is narrower than four choices in one line: each wraps its words instead of the row running off. */
  .seg { align-self: flex-start; max-width: 100%; }
  .seg-btn { flex: 1 1 auto; min-width: 0; height: auto; min-height: 34px; padding-block: 6px; white-space: normal; text-align: center; justify-content: center; line-height: 1.25; }
  .blocked { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-2); }
  .who { display: flex; align-items: center; gap: var(--sp-2); min-width: 0; }
  .who-text { display: flex; flex-direction: column; min-width: 0; }
  .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .npub { font-size: var(--fs-2xs); color: var(--text-3); }
</style>
