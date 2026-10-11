<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Social Recovery (cloud-recovery.md §11): /cloud/help; back leads to /cloud. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import CloudScreen from '$lib/cloud/CloudScreen.svelte';
  import HelpFriend from '$lib/cloud/HelpFriend.svelte';

  const search = typeof window === 'undefined' ? '' : window.location.search;
</script>

<CloudScreen title={$t('cloud_help_title')} back="/cloud">
  <HelpFriend {search} />
</CloudScreen>
