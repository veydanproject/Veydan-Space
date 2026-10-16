<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The plan of the account (vcloud-plan.md 4.2, 4.5): its name and state, the day it is paid until, the plan that waits for that day, the three numbers a person feels (room, a file of a chat, how long media stay), and the two ways to change it: the plans and a key. How old the numbers are is said when the server could not be asked. -->
<script lang="ts">
  import { t, locale } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { isMobile } from '$lib/core/platform';
  import { dateOf, formatBytes, type ShownAccount } from '$lib/cloud/account';
  import Notice from '$lib/cloud/Notice.svelte';

  let {
    account,
    name,
    nextName = '',
    hasPlans = false,
    onPlans,
    onKey,
  }: {
    account: ShownAccount;
    /** The name of the plan as the server calls it. */
    name: string;
    /** The name of the plan that starts when this one ends, if one waits. */
    nextName?: string;
    /** There is a choice of plans on the page to lead to. */
    hasPlans?: boolean;
    onPlans: () => void;
    onKey: () => void;
  } = $props();

  const stateKey = $derived(
    ({ free: 'cloud_state_free', paid: 'cloud_state_paid', grace: 'cloud_state_grace', suspended: 'cloud_state_suspended' } as const)[
      account.state
    ],
  );
  const files = $derived(account.limits['vault.files_bytes']);
  const chatFile = $derived(account.limits['media.max_file_bytes']);
  const retention = $derived(account.limits['media.retention_days']);
</script>

<section class="hero" class:m={isMobile} aria-labelledby="cloud-plan-title">
  <div class="top">
    <span class="tile"><Icon name="cloud" size={22} /></span>
    <div class="words">
      <span class="label" id="cloud-plan-title">{$t('cloud_plan_title')}</span>
      <span class="name">{name}</span>
    </div>
    <span class="badge" class:badge-ok={account.state === 'paid' || account.state === 'free'} class:badge-warn={account.state === 'grace'} class:badge-danger={account.state === 'suspended'}>
      {$t(stateKey)}
    </span>
  </div>

  {#if account.state === 'grace' && account.grace_until}
    <Notice tone="warn">{$t('cloud_grace_until', { date: dateOf(account.grace_until, $locale) })}</Notice>
  {:else if account.state === 'suspended'}
    <Notice tone="danger">{$t('cloud_suspended_text')}</Notice>
  {:else if account.until}
    <p class="line">{$t('cloud_until', { date: dateOf(account.until, $locale) })}</p>
  {/if}
  {#if account.next && nextName}
    <p class="line">{$t('cloud_next_plan', { plan: nextName, date: dateOf(account.next.since, $locale) })}</p>
  {/if}

  {#if files || chatFile || retention}
    <dl class="facts">
      {#if files}<div><dd>{formatBytes(files)}</dd><dt>{$t('cloud_fact_files')}</dt></div>{/if}
      {#if chatFile}<div><dd>{formatBytes(chatFile)}</dd><dt>{$t('cloud_fact_chat_file')}</dt></div>{/if}
      {#if retention}<div><dd>{$t('cloud_fact_days', { days: String(retention) })}</dd><dt>{$t('cloud_fact_retention')}</dt></div>{/if}
    </dl>
  {/if}

  <div class="actions">
    {#if hasPlans}
      <button class="btn" class:btn-primary={account.state !== 'paid'} class:btn-ghost={account.state === 'paid'} type="button" onclick={onPlans}>
        {account.state === 'free' ? $t('cloud_plan_upgrade') : $t('cloud_plan_see_all')}
      </button>
    {/if}
    <button class="btn btn-ghost" type="button" onclick={onKey}><Icon name="key" size={14} />{$t('cloud_key_enter')}</button>
  </div>

  {#if account.keptAt}
    <p class="line muted">{$t('cloud_kept_at', { date: dateOf(account.keptAt, $locale) })}</p>
  {/if}
</section>

<style>
  .hero { display: flex; flex-direction: column; gap: var(--sp-4); }
  .top { display: flex; align-items: center; gap: var(--sp-3); }
  .tile {
    display: inline-flex; align-items: center; justify-content: center; width: 48px; height: 48px; flex-shrink: 0;
    border-radius: var(--radius-md); background: var(--accent-grad); color: #fff; box-shadow: var(--shadow-accent);
  }
  .words { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .label { font-size: var(--fs-xs); font-weight: var(--fw-bold); color: var(--text-dim); text-transform: uppercase; letter-spacing: 1px; }
  .name { font-size: var(--fs-xl); font-weight: var(--fw-extrabold); color: var(--text); line-height: 1.15; letter-spacing: -0.01em; }
  .line { margin: 0; font-size: var(--fs-base); color: var(--text-body); }
  .muted { color: var(--text-3); font-size: var(--fs-sm); }
  .facts { margin: 0; display: grid; grid-auto-flow: column; grid-auto-columns: minmax(0, 1fr); gap: var(--sp-2); }
  .facts div { display: flex; flex-direction: column; gap: 2px; padding: var(--sp-3); border-radius: var(--radius-md); background: var(--surface-2); border: 1px solid var(--border); min-width: 0; }
  .facts dd { margin: 0; font-size: var(--fs-lg); font-weight: var(--fw-bold); color: var(--text); white-space: nowrap; }
  .facts dt { font-size: var(--fs-xs); color: var(--text-2); line-height: 1.35; }
  .actions { display: flex; gap: var(--sp-2); flex-wrap: wrap; }
  .m .actions .btn { flex: 1 1 auto; min-height: 48px; font-size: var(--fs-body); }
  .m .facts div { padding: var(--sp-2) var(--sp-3); }
  .m .facts dd { font-size: var(--fs-md); }
</style>
