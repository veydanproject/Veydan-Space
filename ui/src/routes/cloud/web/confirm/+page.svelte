<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The confirmation of an action of the web cabinet (/cloud/web/confirm, cloud-api.md §16.3): opened by the card `web.confirm` in the chat with the bot; back leads there. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import { leave } from '$lib/core/ui/back';
  import CloudScreen from '$lib/cloud/CloudScreen.svelte';
  import WebConfirm from '$lib/cloud/WebConfirm.svelte';

  const search = typeof window === 'undefined' ? '' : window.location.search;
  const back = () => void leave('/cloud');
</script>

<CloudScreen title={$t('cloud_web_confirm_screen_title')} back="/cloud" onback={back}>
  <WebConfirm {search} onleave={back} />
</CloudScreen>
