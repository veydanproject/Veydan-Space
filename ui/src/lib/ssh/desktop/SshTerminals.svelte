<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The terminal drawers of the open SSH sessions, over the page. The slot ends
  where the shell's bottom bars begin, so a drawer slides up from them; the
  slot clips the animation (overflow: hidden keeps it from sliding through
  the bars).
-->
<script lang="ts">
  import SSHTerminal from '$lib/ssh/components/SSHTerminal.svelte';
  import { sshStore } from '$lib/ssh/store/ssh.svelte';

  let slot = $state<HTMLDivElement | null>(null);

  /** Distance from the bottom of the window to the bottom of the slot, for the drag math of a drawer. */
  function bottomOffset(): number {
    if (!slot) return 0;
    return Math.max(0, Math.round(window.innerHeight - slot.getBoundingClientRect().bottom));
  }
</script>

<div class="ssh-terminal-slot" bind:this={slot}>
  {#each sshStore.sessions as s (s.session_id)}
    <SSHTerminal
      sessionId={s.session_id}
      visible={sshStore.activeTerminalId === s.session_id}
      {bottomOffset}
      onMinimize={() => { sshStore.activeTerminalId = null; }}
      onDisconnect={() => { sshStore.activeTerminalId = null; }}
    />
  {/each}
</div>

<style>
  .ssh-terminal-slot {
    position: absolute;
    inset: 0;
    overflow: hidden;
    pointer-events: none;
    z-index: 50;
  }
</style>
