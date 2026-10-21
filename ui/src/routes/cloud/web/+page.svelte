<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- «Вход на сайт» (/cloud/web, cloud-api.md §16.2): from the page of the cloud, or with `?id=` from a scanned or pasted link in the messenger; back leads to the page it was opened from, /cloud when there is none. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import { leave } from '$lib/core/ui/back';
  import CloudScreen from '$lib/cloud/CloudScreen.svelte';
  import WebLoginApprove from '$lib/cloud/WebLoginApprove.svelte';

  const search = typeof window === 'undefined' ? '' : window.location.search;
  const back = () => void leave('/cloud');
</script>

<CloudScreen title={$t('cloud_web_screen_title')} back="/cloud" onback={back}>
  <WebLoginApprove {search} onleave={back} />
</CloudScreen>
