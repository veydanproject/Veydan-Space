<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The peer's face in a call: waves go out from it while the call rings,
  and a halo follows the peer's voice while it talks.
-->
<script lang="ts">
  import Avatar from '../contacts/Avatar.svelte';
  import type { CallPeer } from './peer';

  interface Props {
    peer: CallPeer;
    size: number;
    /** Waves while it rings (either way). */
    ringing?: boolean;
    /** How loud the peer is now, 0..1; the halo follows it. */
    level?: number;
  }
  let { peer, size, ringing = false, level = 0 }: Props = $props();
</script>

<div class="face" class:ringing style:--size="{size}px" style:--level={Math.max(0, Math.min(1, level)).toFixed(3)}>
  {#if ringing}
    <span class="wave"></span><span class="wave late"></span>
  {:else}
    <span class="halo"></span>
  {/if}
  <div class="pic"><Avatar url={peer.picture} label={peer.name} seed={peer.seed} {size} /></div>
</div>

<style>
  .face { position: relative; width: var(--size); height: var(--size); flex-shrink: 0; }
  .pic { position: relative; border-radius: 50%; }
  .wave, .halo { position: absolute; inset: 0; border-radius: 50%; pointer-events: none; }
  .wave { border: 2px solid var(--accent); animation: wave 2.4s cubic-bezier(0.2, 0.6, 0.35, 1) infinite; }
  .wave.late { animation-delay: 1.2s; }
  .halo {
    background: var(--success);
    opacity: calc(var(--level) * 0.55);
    transform: scale(calc(1 + var(--level) * 0.28));
    /* The voice comes in bursts: the halo eases toward it instead of jumping. */
    transition: transform 160ms ease-out, opacity 160ms ease-out;
  }
  @keyframes wave {
    from { transform: scale(1); opacity: 0.75; }
    to { transform: scale(1.55); opacity: 0; }
  }
  @media (prefers-reduced-motion: reduce) {
    .wave { animation: none; opacity: 0.4; transform: scale(1.12); }
    .wave.late { display: none; }
  }
</style>
