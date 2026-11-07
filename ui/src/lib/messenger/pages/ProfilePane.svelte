<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- My profile, my key and the archive of chats on a phone, without the other tabs of the chat's settings: the screen behind the avatar in Space (mobile/MobileProfile), the first tab of the settings in Chat (entry/mobile, `into: 'tab'`). -->
<script lang="ts">
  import { onMount } from 'svelte';
  import '../shared/emoji/font.css';
  import { markPhone } from '../shared/phone';
  import { messengerStore } from '../store.svelte';
  import OwnProfileCard from '../contacts/OwnProfileCard.svelte';
  import IdentityCard from '../identity/IdentityCard.svelte';
  import ArchiveCard from '../archive/ArchiveCard.svelte';
  import { openSettings, settingsUi } from './settingsTab.svelte';

  markPhone();

  onMount(() => { messengerStore.ensureLoaded().catch(() => {}); });

  // An avatar needs a media server to be stored on: the network tab of the chat's settings, scrolled to them.
  function openMedia() {
    settingsUi.focusMedia = true;
    openSettings('network', true);
  }
</script>

<!-- `msg-m`: the messenger's phone look of the cards inside (OwnProfileCard), as in its own screens. -->
<div class="msg-m profile">
  <!-- No profile card for an identity that is gone (removed a moment ago) -->
  {#if messengerStore.identity}
    <OwnProfileCard onopenmedia={openMedia} />
    <IdentityCard identity={messengerStore.identity} />
    <!-- The archive of chats, as under «Profile and key» of the settings: whichever way the profile is opened. -->
    <ArchiveCard />
  {/if}
</div>

<style>
  .profile { display: flex; flex-direction: column; gap: var(--sp-3); }
  /* The cards of the chat's settings on a phone (SettingsView, compact). */
  .profile :global(.card) {
    max-width: none; width: 100%; padding: var(--sp-4) var(--sp-5);
    background: var(--pane-strong); border-color: var(--m-card-border, var(--border));
    box-shadow: var(--m-card-shadow, none);
  }
</style>
