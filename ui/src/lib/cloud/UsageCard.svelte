<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The space of the account by its pools (vcloud-plan.md 4.5, 4.7): the files of the vault first and large, since that is the pool that fills; the text of the vault and the media of chats under it; what takes the room by category when the storage tells. A pool nearly full says so and leads to more room. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import { FULL_PERCENT, NOTICE_PERCENT, WARN_PERCENT } from '$lib/core/sync/cloud';
  import { formatBytes, poolsOf, type PoolView, type ShownAccount } from '$lib/cloud/account';
  import type { CloudCategory } from '$lib/cloud/usage';

  let {
    account,
    categories = [],
    onMore,
  }: { account: ShownAccount; categories?: CloudCategory[]; /** The way to more room, when the page has one. */ onMore?: () => void } = $props();

  const pools = $derived(poolsOf(account));
  const files = $derived(pools.find((p) => p.id === 'files') ?? null);
  const rest = $derived(pools.filter((p) => p.id !== 'files'));
  const titles = { files: 'cloud_pool_files', text: 'cloud_pool_text', media: 'cloud_pool_media' } as const;
  const retention = $derived(account.limits['media.retention_days']);
  const shownCategories = $derived(categories.filter((c) => c.bytes > 0).sort((a, b) => b.bytes - a.bytes));

  const CATEGORY = {
    journal: 'cloud_quota_cat_journal',
    files: 'cloud_quota_cat_files',
    large_files: 'cloud_quota_cat_large_files',
    recovery: 'cloud_quota_cat_recovery',
    other: 'cloud_quota_cat_other',
  } as const;

  function amount(p: PoolView): string {
    if (p.limit !== null) return $t('cloud_pool_of', { used: formatBytes(p.used), limit: formatBytes(p.limit) });
    if (p.id === 'media' && retention) return $t('cloud_pool_media_kept', { used: formatBytes(p.used), days: String(retention) });
    return $t('cloud_pool_nolimit', { used: formatBytes(p.used) });
  }

  /** The width of the line of a pool: an empty pool has none, a pool with anything in it shows at least a sliver. */
  function fill(percent: number): number {
    return percent > 0 ? Math.max(percent, 1) : 0;
  }

  /** The tone of a pool by how full it is: the levels of the core, which the banner and the sync settings share. */
  function tone(percent: number | null): '' | 'warn' | 'full' {
    if (percent === null) return '';
    return percent >= WARN_PERCENT ? 'full' : percent >= NOTICE_PERCENT ? 'warn' : '';
  }
</script>

<section class="usage" aria-labelledby="cloud-usage-title">
  <h2 id="cloud-usage-title">{$t('cloud_usage_title')}</h2>

  {#if files}
    <div class="main {tone(files.percent)}">
      <div class="figures">
        <span class="used">{formatBytes(files.used)}</span>
        {#if files.limit !== null}<span class="of">{$t('cloud_usage_of', { limit: formatBytes(files.limit) })}</span>{/if}
        {#if files.percent !== null}<span class="percent">{files.percent}%</span>{/if}
      </div>
      {#if files.percent !== null}
        <div class="bar big" role="progressbar" aria-label={$t(titles.files)} aria-valuemin={0} aria-valuemax={100} aria-valuenow={files.percent}>
          <span class="fill" style:width="{fill(files.percent)}%"></span>
        </div>
      {/if}
      <span class="caption">{$t(titles.files)}</span>
      {#if tone(files.percent) === 'full' && onMore}
        <div class="more">
          <span>{(files.percent ?? 0) >= FULL_PERCENT ? $t('cloud_quota_full_files') : $t('cloud_usage_almost_full')}</span>
          <button class="btn btn-primary btn-sm" type="button" onclick={onMore}>{$t('cloud_quota_more')}</button>
        </div>
      {/if}
    </div>
  {/if}

  <ul class="pools">
    {#each rest as p (p.id)}
      <li class={tone(p.percent)}>
        <div class="row">
          <span>{$t(titles[p.id])}</span>
          <span class="amount">{amount(p)}</span>
        </div>
        {#if p.percent !== null}
          <div class="bar" role="progressbar" aria-label={$t(titles[p.id])} aria-valuemin={0} aria-valuemax={100} aria-valuenow={p.percent}>
            <span class="fill" style:width="{fill(p.percent)}%"></span>
          </div>
          <!-- The pool of the text full is the one that stops the sync of notes and passwords: said in words, not by a red line alone. -->
          {#if p.id === 'text' && p.percent >= FULL_PERCENT}<span class="stopped">{$t('cloud_quota_full_text')}</span>{/if}
        {/if}
      </li>
    {/each}
  </ul>

  {#if shownCategories.length > 0}
    <div class="what">
      <span class="section-label">{$t('cloud_quota_what')}</span>
      <ul>
        {#each shownCategories as c (c.id)}
          <li><span>{$t(CATEGORY[c.id] ?? CATEGORY.other)}</span><span class="amount mono">{formatBytes(c.bytes)}</span></li>
        {/each}
      </ul>
    </div>
  {/if}
</section>

<style>
  .usage { display: flex; flex-direction: column; gap: var(--sp-4); }
  h2 { margin: 0; font-size: var(--fs-md); font-weight: var(--fw-bold); }
  .main { display: flex; flex-direction: column; gap: var(--sp-2); }
  .figures { display: flex; align-items: baseline; gap: var(--sp-2); }
  .used { font-size: var(--fs-xl); font-weight: var(--fw-extrabold); color: var(--text); letter-spacing: -0.01em; }
  .of { font-size: var(--fs-base); color: var(--text-2); }
  .percent { margin-left: auto; font-size: var(--fs-base); font-weight: var(--fw-semibold); color: var(--text-2); }
  .caption { font-size: var(--fs-sm); color: var(--text-2); }
  .bar { height: 6px; border-radius: var(--radius-pill); background: var(--surface-3); overflow: hidden; }
  .bar.big { height: 10px; }
  .fill { display: block; height: 100%; border-radius: var(--radius-pill); background: var(--accent-grad); }
  .warn .fill { background: var(--warn); }
  .full .fill { background: var(--danger); }
  .full .percent { color: var(--danger-text); }
  .more {
    display: flex; align-items: center; justify-content: space-between; gap: var(--sp-3); flex-wrap: wrap;
    padding: var(--sp-3); border-radius: var(--radius-md); background: var(--danger-bg); border: 1px solid var(--danger-border);
    font-size: var(--fs-sm); color: var(--danger-text);
  }
  .pools { list-style: none; margin: 0; padding: var(--sp-4) 0 0; border-top: 1px solid var(--border); display: flex; flex-direction: column; gap: var(--sp-3); }
  .pools li { display: flex; flex-direction: column; gap: 6px; }
  .row { display: flex; justify-content: space-between; gap: var(--sp-3); font-size: var(--fs-base); color: var(--text-body); }
  .amount { color: var(--text-2); white-space: nowrap; font-size: var(--fs-sm); }
  .stopped { font-size: var(--fs-sm); color: var(--danger-text); line-height: 1.4; }
  .what { display: flex; flex-direction: column; gap: var(--sp-2); padding-top: var(--sp-4); border-top: 1px solid var(--border); }
  .what ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .what li { display: flex; justify-content: space-between; gap: var(--sp-3); font-size: var(--fs-base); color: var(--text-body); }
</style>
