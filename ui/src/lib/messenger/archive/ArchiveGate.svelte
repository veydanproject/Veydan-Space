<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Step B of the restore of chats (messenger-archive-plan.md section 5): the
  screen that holds the first minutes after a restore by 12 words or a
  device linking, while the newest months of the archive come back. A bar
  with the real number of bytes, the month under way, an estimate. It lets go
  after 20 s (the restore goes on behind it; the banner of the chats takes
  over) or when there is nothing to restore. A window overlay of the module,
  so it stands over whichever screen the restore ended on.
-->
<script lang="ts">
  import { t, locale } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { messengerStore } from '../store.svelte';
  import { archiveStore } from './archiveStore.svelte';
  import { etaParts, monthLabel } from './archive';

  const month = $derived(monthLabel(archiveStore.progress.month, $locale));
  const eta = $derived(archiveStore.eta === null ? null : etaParts(archiveStore.eta));

  // The page that restored the account has no identity of the module yet: read it, and the status with it.
  $effect(() => {
    if (archiveStore.armed && !messengerStore.identity) messengerStore.refresh().catch(() => {});
  });
</script>

{#if archiveStore.blocking}
  <div class="gate" role="dialog" aria-modal="true" aria-busy="true" aria-labelledby="archive-gate-title">
    <div class="box">
      <span class="tile"><Icon name="archive-restore" size={28} /></span>
      <h1 id="archive-gate-title">{$t('msg_archive_unpack_title')}</h1>
      <div class="meter">
        <!-- DS-STUB: progress line -->
        <div class="bar" role="progressbar" aria-label={$t('msg_archive_unpack_title')} aria-valuemin={0} aria-valuemax={100} aria-valuenow={archiveStore.percent}>
          <span class="fill" style:width="{archiveStore.percent}%"></span>
        </div>
        <div class="figures">
          <span class="percent">{archiveStore.percent} %</span>
          {#if eta}<span class="eta">{$t(eta.unit === 'min' ? 'msg_archive_unpack_eta_min' : 'msg_archive_unpack_eta', { n: String(eta.n) })}</span>{/if}
        </div>
      </div>
      {#if month}<p class="month">{$t('msg_archive_unpack_month', { month })}</p>{/if}
      <p class="keep">{$t('msg_archive_unpack_keep')}</p>
    </div>
  </div>
{/if}

<style>
  .gate {
    position: fixed; inset: var(--overlay-inset, 0); border-radius: var(--overlay-radius, 0);
    /* Over the wizard and the sheets of a phone, under nothing: the app is not to be used yet. */
    z-index: calc(var(--z-modal) + 1);
    display: flex; align-items: center; justify-content: center;
    padding: calc(var(--sat, 0px) + var(--sp-4)) var(--sp-4) calc(var(--sab, 0px) + var(--sp-4));
    background: var(--bg-image), var(--bg); color: var(--text);
  }
  .box { width: min(100%, 420px); display: flex; flex-direction: column; align-items: center; gap: var(--sp-4); text-align: center; }
  .tile {
    display: inline-flex; align-items: center; justify-content: center; width: 56px; height: 56px;
    border-radius: var(--radius-md); background: var(--accent-bg); color: var(--accent-text);
  }
  h1 { margin: 0; font-size: var(--fs-xl); font-weight: var(--fw-extrabold); line-height: var(--lh-snug); }
  .meter { width: 100%; display: flex; flex-direction: column; gap: var(--sp-2); }
  .bar { height: 10px; border-radius: var(--radius-pill); background: var(--surface-3); overflow: hidden; }
  .fill { display: block; height: 100%; border-radius: var(--radius-pill); background: var(--accent-grad); transition: width 0.4s ease; }
  .figures { display: flex; justify-content: space-between; gap: var(--sp-3); font-size: var(--fs-sm); color: var(--text-2); }
  .percent { font-weight: var(--fw-semibold); color: var(--text); }
  .month { margin: 0; font-size: var(--fs-base); color: var(--text-2); }
  .keep { margin: 0; font-size: var(--fs-sm); color: var(--text-3); }
</style>
