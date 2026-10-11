<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Social Recovery (cloud-recovery.md §11): /cloud/trustees; back leads to /cloud. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import CloudScreen from '$lib/cloud/CloudScreen.svelte';
  import Trustees from '$lib/cloud/Trustees.svelte';
</script>

<CloudScreen title={$t('cloud_trustees_title')} back="/cloud">
  <Trustees />
</CloudScreen>
