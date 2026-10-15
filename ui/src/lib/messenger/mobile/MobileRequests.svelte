<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The phone's screen of the requests of strangers: a tap opens one on a screen of its own. -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import { leave } from '$lib/core/ui/back';
  import { messengerStore } from '../store.svelte';
  import RequestsList from '../requests/RequestsList.svelte';
  import MobileFrame from './MobileFrame.svelte';
  import { BASE, requestHref } from './routes';
  import { markPhone } from '../shared/phone';

  markPhone();

  onMount(() => {
    messengerStore.ensureLoaded().catch(() => {});
    messengerStore.startListeners().catch(() => {});
  });
</script>

<MobileFrame title={$t('msg_req_title')} onback={() => void leave(BASE)} scroll={false}>
  <RequestsList onopen={(r) => goto(requestHref(r.peer))} />
</MobileFrame>
