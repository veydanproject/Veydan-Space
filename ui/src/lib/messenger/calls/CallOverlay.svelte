<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Calls on a computer, mounted once by the shell while the messenger is
  visible: the card of a call that rings, the panel of the call under
  way, their sounds; the room of a group call (its window, or its capsule
  when folded). The system's notification of a ringing call (when
  the window is away) is the host's; this is the window's own.
-->
<script lang="ts">
  import { messengerStore } from '../store.svelte';
  import { openFromNotice } from '../push/desktop';
  import CallPanel from './CallPanel.svelte';
  import CallSounds from './CallSounds.svelte';
  import IncomingCall from './IncomingCall.svelte';
  import VideoWindow from './VideoWindow.svelte';
  import GroupCallPanel from './GroupCallPanel.svelte';
  import GroupCallWindow from './GroupCallWindow.svelte';
</script>

{#if messengerStore.visible}
  <IncomingCall />
  <VideoWindow />
  <CallPanel onchat={(chatId) => openFromNotice(chatId).catch(() => {})} />
  <CallSounds />
  <GroupCallWindow onchat={(chatId) => openFromNotice(chatId).catch(() => {})} />
  <GroupCallPanel onchat={(chatId) => openFromNotice(chatId).catch(() => {})} />
{/if}
