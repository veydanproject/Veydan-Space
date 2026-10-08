<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  What a call sounds like in the page, for as long as it is mounted: the
  ring of a call that comes in, the caller's motif while the peer's device
  rings, a chime when the call connects, a falling one when it ends, and a
  "no" when the peer declines, is busy or does not answer (sounds.ts picks,
  tones.ts plays). `ring` off: something else rings for the page (the
  phone's call service and its notification).
-->
<script lang="ts">
  import { callStore } from './callStore.svelte';
  import { connectedSound, endSound } from './sounds';
  import { busyTone, connectedTone, endTone, ringback, ringtone } from './tones';

  let { ring = true }: { ring?: boolean } = $props();

  const phase = $derived(callStore.call?.phase ?? null);
  const sound = $derived(phase === 'incoming' && ring ? 'ring' : phase === 'outgoing' ? 'back' : null);

  $effect(() => {
    if (!sound) return;
    return sound === 'ring' ? ringtone() : ringback();
  });

  // The first time a call talks: a chime, once (not again after a reconnection).
  let chimed = '';
  $effect(() => {
    const c = callStore.call;
    if (!c || c.phase !== 'active' || c.call_id === chimed) return;
    chimed = c.call_id;
    if (connectedSound(c, Math.floor(Date.now() / 1000))) connectedTone();
  });

  // A call that ended: once.
  let sounded = '';
  $effect(() => {
    const e = callStore.ended;
    if (!e || e.call.call_id === sounded) return;
    sounded = e.call.call_id;
    const s = endSound(e);
    if (s === 'end') endTone();
    else if (s === 'busy') busyTone();
  });
</script>
