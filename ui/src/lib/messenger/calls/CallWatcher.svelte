<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Calls on a phone, mounted once by the shell: every call that rings, is
  made or is found taken (the call service's Answer started the app) opens
  the call's page once; left for another screen, the call stays one tap
  away in a bar at the top. The room of a group call I start or join opens
  its own page the same way. The ring of a call that comes in is the call
  service's on the phone; the page sounds only the line's tone while I call.
-->
<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { t } from '$lib/core/i18n';
  import { messengerStore } from '../store.svelte';
  import { callHref, groupCallHref } from '../mobile/routes';
  import CallIcon from './CallIcon.svelte';
  import CallSounds from './CallSounds.svelte';
  import { callStore } from './callStore.svelte';
  import { groupStatusText } from './group';
  import { groupCallStore } from './groupCallStore.svelte';
  import { groupStore } from '../groups/groupStore.svelte';
  import { callPeer } from './peer';
  import { statusText } from './status';

  const tr = (key: string, params?: Record<string, string>) => $t(key as 'msg_title', params);

  const onCallPage = $derived(page.url.pathname === callHref());
  const call = $derived(callStore.call);

  // Each call opens its page once; after that the user decides where to be.
  // A call that rings while I sit in a room cannot be taken (the bar below
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

  const bar = $derived(messengerStore.visible && call && !onCallPage ? call : null);
  const peer = $derived(bar ? callPeer(bar) : null);
  const status = $derived(bar ? statusText(bar, null, callStore.now, tr) : '');

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
  const roomBar = $derived(messengerStore.visible && room && !onRoomPage && !bar ? room : null);
  const roomName = $derived(roomBar ? (groupStore.groups[roomBar.group_id]?.name ?? '') : '');
  const roomStatus = $derived(roomBar ? groupStatusText(roomBar, null, groupCallStore.now, tr) : '');
</script>

{#if messengerStore.visible}
  <CallSounds ring={false} />
{/if}

{#if bar && peer}
  <button class="back-to-call" aria-label={$t('msg_call_return')} title={$t('msg_call_return')} class:ringing={bar.phase === 'incoming'} onclick={() => goto(callHref())}>
    <CallIcon name={bar.phase === 'incoming' ? 'incoming' : 'phone'} size={15} />
    <span class="who">{peer.name}</span>
    <span class="what">{status}</span>
  </button>
{/if}

{#if roomBar}
  <button class="back-to-call" aria-label={$t('msg_call_return')} title={$t('msg_call_return')} onclick={() => goto(groupCallHref())}>
    <CallIcon name="phone" size={15} />
    <span class="who">{roomName}</span>
    <span class="what">{roomStatus}</span>
  </button>
{/if}

<style>
  /* A capsule just under the top bar of the screen below: over the start of
     what scrolls, never over the bar's name, back button and actions. */
  .back-to-call {
    position: fixed; z-index: 75; top: calc(var(--sat, 0px) + 66px); left: 50%; transform: translateX(-50%);
    max-width: calc(100vw - 48px); display: flex; align-items: center; gap: 6px;
    padding: 7px 14px; border: none; border-radius: var(--radius-pill); cursor: pointer;
    background: var(--success); color: #fff; font: inherit; font-size: var(--fs-xs); box-shadow: var(--shadow-lg);
  }
  .back-to-call.ringing { background: var(--accent); }
  .who { font-weight: var(--fw-bold); min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .what { font-variant-numeric: tabular-nums; opacity: 0.9; white-space: nowrap; }
</style>
