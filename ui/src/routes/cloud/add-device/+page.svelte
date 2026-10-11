<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- «Добавить устройство» (/cloud/add-device): opened from the page of the cloud, so back leads there. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import CloudScreen from '$lib/cloud/CloudScreen.svelte';
  import LinkAddDevice from '$lib/cloud/LinkAddDevice.svelte';
</script>

<CloudScreen title={$t('cloud_link_add_title')} back="/cloud">
  <LinkAddDevice />
</CloudScreen>
