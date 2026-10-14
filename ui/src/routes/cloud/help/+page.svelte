<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Social Recovery (cloud-recovery.md §11): /cloud/help; back leads to the page it was opened from (a chat, through the card of a request), /cloud when there is none. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import { leave } from '$lib/core/ui/back';
  import CloudScreen from '$lib/cloud/CloudScreen.svelte';
  import HelpFriend from '$lib/cloud/HelpFriend.svelte';

  const search = typeof window === 'undefined' ? '' : window.location.search;
  const back = () => void leave('/cloud');
</script>

<CloudScreen title={$t('cloud_help_title')} back="/cloud" onback={back}>
  <HelpFriend {search} onleave={back} />
</CloudScreen>
