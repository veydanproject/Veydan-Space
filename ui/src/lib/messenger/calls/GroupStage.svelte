<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The seats of a group call, filling the box it is put in (the desk's
  window, the phone's page). A grid of equals while nobody's video is to
  be seen large; the video of who speaks (or of the seat a tap picked)
  large with the others in a row under it otherwise; a tap picks only a
  seat whose camera is on (a voice gains nothing large). The large place
  by the voices is held a while against a quick change, and none of it
  moves while a camera that is on sends no frames for a moment
  (groupCallStore `voice`, `showing`). Each tile asks the node for the
  layer of video its size needs (GroupTile).
-->
<script lang="ts">
  import type { GroupCallView } from '../api';
  import { gridShape, mirrorsMine, seatSendsVideo, seatsInOrder } from './group';
  import { groupCallStore } from './groupCallStore.svelte';
  import GroupTile from './GroupTile.svelte';

  interface Props {
    call: GroupCallView;
    /** The phone's page: a narrower row, room for the bars over it. */
    phone?: boolean;
  }
  let { call, phone = false }: Props = $props();

  /** The seat picked by a tap for the large place. */
  let pinned = $state<number | null>(null);
  let w = $state(0);
  let h = $state(0);

  const seats = $derived(seatsInOrder(call.participants));
  const showing = (seat: number) => !!groupCallStore.showing[seat];
  /** The seat a tap picked while it is there; the one the voices give otherwise. */
  const focus = $derived(pinned != null && call.participants.some((p) => p.id === pinned) ? pinned : groupCallStore.voice);
  const main = $derived(focus != null ? seats.find((p) => p.id === focus) ?? null : null);
  const rest = $derived(main ? seats.filter((p) => p.id !== main.id) : seats);
  const shape = $derived(gridShape(seats.length, w, h, phone ? 3 / 4 : 16 / 10, 8));
  /** My own tile: a face in a mirror; a screen or the world of a back camera as it is. */
  const mirror = $derived(mirrorsMine(call, groupCallStore.screen));

  // A picked seat that left lets the large place go.
  $effect(() => {
    if (pinned != null && !call.participants.some((p) => p.id === pinned)) pinned = null;
  });

  const pick = (id: number) => () => { pinned = pinned === id ? null : id; };
</script>

<div class="stage" class:phone class:focused={!!main} bind:clientWidth={w} bind:clientHeight={h}>
  {#if main}
    <div class="main">
      <!-- A tile of its own for each seat: what a tile holds of its seat (its picture, its size) is not the next one's. -->
      {#key main.id}
        <GroupTile p={main} callId={call.call_id} level={groupCallStore.levels[main.id] ?? 0} big pinned={pinned === main.id}
          muted={call.muted} sending={call.video_local} {mirror} onpick={main.me ? undefined : pick(main.id)} />
      {/key}
    </div>
    <div class="strip">
      {#each rest as p (p.id)}
        <div class="thumb">
          <GroupTile {p} callId={call.call_id} level={groupCallStore.levels[p.id] ?? 0} pinned={pinned === p.id}
            muted={call.muted} sending={call.video_local} {mirror} onpick={p.me || !seatSendsVideo(p, call, showing) ? undefined : pick(p.id)} />
        </div>
      {/each}
    </div>
  {:else}
    <div class="grid" style:--cols={shape.cols} style:--rows={shape.rows}>
      {#each seats as p (p.id)}
        <div class="cell">
          <GroupTile {p} callId={call.call_id} level={groupCallStore.levels[p.id] ?? 0} big={seats.length <= 2} pinned={pinned === p.id}
            muted={call.muted} sending={call.video_local} {mirror} onpick={p.me || !seatSendsVideo(p, call, showing) ? undefined : pick(p.id)} />
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .stage { position: absolute; inset: 0; padding: 8px; box-sizing: border-box; background: #0b0d10; display: flex; flex-direction: column; gap: 8px; }
  .grid {
    flex: 1; min-height: 0; display: grid; gap: 8px;
    grid-template-columns: repeat(var(--cols), minmax(0, 1fr)); grid-template-rows: repeat(var(--rows), minmax(0, 1fr));
  }
  .cell { min-width: 0; min-height: 0; }
  .main { flex: 1; min-height: 0; }
  .strip { display: flex; gap: 8px; overflow-x: auto; flex-shrink: 0; scrollbar-width: thin; }
  .thumb { flex: 0 0 auto; width: clamp(112px, 18%, 180px); aspect-ratio: 16 / 10; }
  .phone .thumb { width: 92px; aspect-ratio: 3 / 4; }
</style>
