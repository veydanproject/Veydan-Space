<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Social Recovery (cloud-recovery.md §11): /cloud/recover; back leads to /messenger. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import CloudScreen from '$lib/cloud/CloudScreen.svelte';
  import RecoverTrusted from '$lib/cloud/RecoverTrusted.svelte';
</script>

<CloudScreen title={$t('cloud_recover_title')} back="/messenger">
  <RecoverTrusted />
</CloudScreen>
