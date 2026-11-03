<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- «Профиль и ключ» alone, behind my avatar in Space: the chat's other settings are in the app's settings (ROO-558). -->
<script lang="ts">
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import { leave } from '$lib/core/ui/back';
  import { messengerStore } from '../store.svelte';
  import ProfilePane from '../pages/ProfilePane.svelte';
  import MobileFrame from './MobileFrame.svelte';
  import { BASE } from './routes';

  // The identity was removed here: the messenger's root shows the onboarding.
  $effect(() => {
    if (messengerStore.loaded && !messengerStore.identity) void goto(BASE, { replaceState: true });
  });
</script>

<!-- Back to the tab the avatar was tapped on. -->
<MobileFrame title={$t('msg_settings_tab_profile')} onback={() => void leave(BASE)}>
  <ProfilePane />
</MobileFrame>
