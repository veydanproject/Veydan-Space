<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { goto } from '$app/navigation';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/mobile/i18n';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';

  let { open, onclose }: { open: boolean; onclose: () => void } = $props();

  function go(href: string) {
    onclose();
    void goto(href);
  }
</script>

<BottomSheet {open} title={$t('nav_more')} {onclose}>
  <div class="m-list">
    <button type="button" class="m-row" onclick={() => go('/notes?kind=archived')}>
      <Icon name="archive" size={20} /><span class="m-row-label">{$t('notes_filter_archived')}</span>
      <span class="chev"><Icon name="chevron-right" size={16} /></span>
    </button>
    <button type="button" class="m-row" onclick={() => go('/notes?kind=trash')}>
      <Icon name="trash-2" size={20} /><span class="m-row-label">{$t('notes_filter_trash')}</span>
      <span class="chev"><Icon name="chevron-right" size={16} /></span>
    </button>
  </div>
  <div class="m-list">
    <button type="button" class="m-row" onclick={() => go('/settings/sync')}>
      <Icon name="refresh-cw" size={20} /><span class="m-row-label">{$t('notes_sync')}</span>
      <span class="chev"><Icon name="chevron-right" size={16} /></span>
    </button>
    <button type="button" class="m-row" onclick={() => go('/settings')}>
      <Icon name="settings" size={20} /><span class="m-row-label">{$t('app_settings')}</span>
      <span class="chev"><Icon name="chevron-right" size={16} /></span>
    </button>
  </div>
</BottomSheet>
