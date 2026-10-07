<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  What a call sounds like in the page, for as long as it is mounted: the
  ring of a call that comes in, the line's tone while the peer's device
  rings, three beeps when a talk ends. `ring` off: something else rings
  for the page (the phone's call service and its notification).
-->
<script lang="ts">
  import { callStore } from './callStore.svelte';
  import { hangupTone, ringback, ringtone } from './tones';

  let { ring = true }: { ring?: boolean } = $props();

  const phase = $derived(callStore.call?.phase ?? null);
  const sound = $derived(phase === 'incoming' && ring ? 'ring' : phase === 'outgoing' ? 'back' : null);

  $effect(() => {
    if (!sound) return;
    return sound === 'ring' ? ringtone() : ringback();
  });

  // A talk that ended, or a call the peer refused: beeps once.
  let beeped = '';
  $effect(() => {
    const e = callStore.ended;
    if (!e || e.call.call_id === beeped) return;
    beeped = e.call.call_id;
    const refused = e.call.direction === 'out' && (e.outcome === 'declined' || e.outcome === 'busy');
    if (e.call.answered_at || refused) hangupTone();
  });
</script>
