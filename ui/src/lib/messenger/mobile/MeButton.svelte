<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- My avatar in the bar of the phone's tabs: my profile and the settings of the chat. -->
<script lang="ts">
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import Avatar from '../contacts/Avatar.svelte';
  import { profileLabel } from '../api';
  import { messengerStore } from '../store.svelte';
  import { BASE } from './routes';
</script>

<button class="me" onclick={() => goto(`${BASE}/settings`)} aria-label={$t('msg_settings_title_full')}>
  <Avatar url={messengerStore.ownProfile?.picture ?? null} label={profileLabel(messengerStore.ownProfile) || $t('msg_you')} seed={messengerStore.identity?.pubkey ?? null} size={34} />
</button>

<style>
  .me { display: inline-flex; padding: 5px; margin-right: 2px; border: none; border-radius: 50%; background: none; cursor: pointer; }
  .me:active { opacity: 0.8; }
</style>
