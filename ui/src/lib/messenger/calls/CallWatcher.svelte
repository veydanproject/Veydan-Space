<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Calls on a phone, mounted once by the shell: every call that rings, is
  made or is found taken (the call service's Answer started the app) opens
  the call's page once; left for another screen, the call stays one tap
  away in a row under the head (CallReturn). The room of a group call I start or join opens
  its own page the same way. The ring of a call that comes in is the call
  service's on the phone; the page sounds only the line's tone while I call.
-->
<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { messengerStore } from '../store.svelte';
  import { callHref, groupCallHref } from '../mobile/routes';
  import CallSounds from './CallSounds.svelte';
  import { callStore } from './callStore.svelte';
  import { groupCallStore } from './groupCallStore.svelte';
  import CallReturn from './CallReturn.svelte';
  import { callReturn } from './returnHosts.svelte';

  const onCallPage = $derived(page.url.pathname === callHref());
  const call = $derived(callStore.call);

  // Each call opens its page once; after that the user decides where to be.
  // A call that rings while I sit in a room cannot be taken (the row of CallReturn
  // shows it, a tap on it refuses it): the room keeps the screen, and the
  // call's page opens once I am out of the room, if it still rings then.
  let opened = '';
  $effect(() => {
    const c = call;
    if (!c || c.call_id === opened) return;
    if (c.phase === 'incoming' && callStore.busyWithGroup) return;
    opened = c.call_id;
    if (!onCallPage) void goto(callHref());
  });

  // The room of a group call: its page opens once per room, at its first word.
  const onRoomPage = $derived(page.url.pathname === groupCallHref());
  const room = $derived(groupCallStore.call);
  let openedRoom = '';
  $effect(() => {
    const r = room;
    if (!r || r.call_id === openedRoom) return;
    openedRoom = r.call_id;
    if (!onRoomPage) void goto(groupCallHref());
  });
</script>

{#if messengerStore.visible}
  <CallSounds ring={false} />
{/if}

<!-- «Return to the call» is a row of the messenger's screens (CallReturn under their head); on a screen with none, here. -->
{#if callReturn.hosts === 0}
  <CallReturn fixed />
{/if}
