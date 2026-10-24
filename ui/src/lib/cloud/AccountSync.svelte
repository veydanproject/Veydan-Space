<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Sync on the page «Account»: where the data of this device goes and whether it does, and the devices the storage knows. The row leads to the settings of sync, where the storage is chosen and connected: this page only says how it stands. -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type SyncConfig, type SyncStatus } from '$lib/core/api';
  import { t, locale, type TranslationKey } from '$lib/core/i18n';
  import { SYNC_HREF } from '$lib/core/onboarding/setup';
  import { formatDateTime } from '$lib/core/utils';
  import NavRow from '$lib/cloud/NavRow.svelte';
  import RowList from '$lib/cloud/RowList.svelte';

  /** The anchor of the block (`?section=sync` of the page «Account»); none inside the settings, whose own section is #sync. */
  let { anchor = 'sync' }: { anchor?: string } = $props();

  let status = $state<SyncStatus | null>(null);
  let cfg = $state<SyncConfig | null>(null);

  onMount(async () => {
    try {
      [status, cfg] = await Promise.all([api.sync.status(), api.sync.getConfig()]);
    } catch {
      // A product whose sync does not answer: the row still leads to its settings.
    }
  });

  const BACKENDS: Record<string, TranslationKey> = {
    cloud: 'settings_sync_backend_cloud',
    s3: 'settings_sync_backend_s3',
    webdav: 'settings_sync_backend_webdav',
    folder: 'settings_sync_backend_folder',
  };

  const line = $derived.by(() => {
    // Not asked yet: nothing is said, so the row does not say «not connected» for a moment.
    if (!status) return '';
    if (!status.joined) return $t('cloud_account_sync_off');
    const where = cfg && BACKENDS[cfg.backend] ? $t(BACKENDS[cfg.backend]) : '';
    const when = status.running
      ? $t('settings_sync_running')
      : status.last_run
        ? $t('cloud_account_sync_last', { when: formatDateTime(status.last_run, $locale) })
        : '';
    return [where, when].filter(Boolean).join(' · ');
  });
  const devices = $derived(
    (status?.joined ? status.storage_devices : []).map((d) => d.name || $t('settings_sync_device_unnamed')).join(', '),
  );
</script>

<RowList label={$t('settings_sync_section')} id={anchor || undefined}>
  <NavRow icon={status?.joined ? 'refresh-cw' : 'cloud'} title={$t('settings_sync_backend')} sub={line} href={SYNC_HREF} tone={status && !status.joined ? 'warn' : undefined} />
  {#if devices}
    <NavRow icon="smartphone" title={$t('settings_sync_storage_devices')} sub={devices} />
  {/if}
</RowList>
