<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- «Восстановить аккаунт» by the 12 words (/cloud/restore): opened from the first screen of the messenger, so back leads there. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import CloudScreen from '$lib/cloud/CloudScreen.svelte';
  import RestoreAccount from '$lib/cloud/RestoreAccount.svelte';
</script>

<CloudScreen title={$t('cloud_restore_title')} back="/messenger">
  <RestoreAccount />
</CloudScreen>
