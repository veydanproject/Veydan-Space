<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- «Войти с другого устройства» (/cloud/link): opened from the first screen of the messenger, so back leads there. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import CloudScreen from '$lib/cloud/CloudScreen.svelte';
  import LinkNewDevice from '$lib/cloud/LinkNewDevice.svelte';
</script>

<CloudScreen title={$t('cloud_link_new_title')} back="/messenger">
  <LinkNewDevice />
</CloudScreen>
