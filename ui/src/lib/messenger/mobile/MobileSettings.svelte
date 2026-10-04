<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import { messengerStore } from '../store.svelte';
  import SettingsView from '../pages/SettingsView.svelte';
  import MobileFrame from './MobileFrame.svelte';
  import { BASE } from './routes';
  import { markPhone } from '../shared/phone';

  markPhone();

  onMount(() => { messengerStore.ensureLoaded().catch(() => {}); });

  // The identity was removed here: the messenger's root shows the onboarding.
  $effect(() => {
    if (messengerStore.loaded && !messengerStore.identity) void goto(BASE, { replaceState: true });
  });
</script>

<!-- "Settings" alone is the app's own tab: this screen is the messenger's. -->
<MobileFrame title={$t('msg_settings_title_full')} onback={() => goto(BASE)}>
  <SettingsView compact />
</MobileFrame>
