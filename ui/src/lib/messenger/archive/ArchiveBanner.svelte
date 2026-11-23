<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Step C of the restore of chats: the strip above the list while the older
  months come back (messenger-archive-plan.md section 5). The shape of the
  strips under a header (design system, "Полосы под шапкой"): the incoming
  bubble's fill, a tile with the sign, a thin line of progress under the text.
  It goes with `archive.synced`.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { archiveStore } from './archiveStore.svelte';
  import { yearsLabel } from './archive';

  const years = $derived(yearsLabel(archiveStore.status?.from_month, archiveStore.status?.to_month ?? archiveStore.progress.month));
  const text = $derived(
    years
      ? $t('msg_archive_banner', { years, percent: String(archiveStore.percent) })
      : $t('msg_archive_banner_plain', { percent: String(archiveStore.percent) }),
  );
</script>

{#if archiveStore.restoring && !archiveStore.blocking}
  <div class="strip" role="status">
    <span class="tile"><Icon name="archive-restore" size={16} /></span>
    <div class="body">
      <span class="text">{text}</span>
      <!-- DS-STUB: progress line -->
      <div class="bar" role="progressbar" aria-label={$t('msg_archive_title')} aria-valuemin={0} aria-valuemax={100} aria-valuenow={archiveStore.percent}>
        <span class="fill" style:width="{archiveStore.percent}%"></span>
      </div>
    </div>
  </div>
{/if}

<style>
  .strip {
    display: flex; align-items: center; gap: var(--sp-3); min-height: 44px; margin: var(--sp-2) 0;
    padding: var(--sp-2) var(--sp-3); border-radius: var(--radius-lg);
    background: var(--bubble-in-bg); border: 1px solid var(--bubble-in-border);
    font-size: var(--fs-sm); color: var(--text-2);
  }
  .tile {
    flex-shrink: 0; display: inline-flex; align-items: center; justify-content: center; width: 28px; height: 28px;
    border-radius: var(--radius-sm); background: var(--accent-bg); color: var(--accent-text);
  }
  .body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 6px; }
  .text { line-height: var(--lh-snug); }
  .bar { height: 3px; border-radius: var(--radius-pill); background: var(--surface-3); overflow: hidden; }
  .fill { display: block; height: 100%; border-radius: var(--radius-pill); background: var(--accent-grad); transition: width 0.4s ease; }
</style>
