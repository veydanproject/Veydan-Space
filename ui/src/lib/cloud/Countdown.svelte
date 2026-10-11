<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- How long a code or a request still lives: the time and a line that runs out with it. -->
<script lang="ts">
  import { timeLeft } from '$lib/cloud/link.svelte';

  let { expiresAt, total, now, label }: { expiresAt: number; total: number; now: number; label: (time: string) => string } = $props();

  const left = $derived(Math.max(0, expiresAt - now));
  const share = $derived(total > 0 ? Math.min(100, (left / total) * 100) : 0);
</script>

<div class="countdown" class:low={left <= 60}>
  <span class="text">{label(timeLeft(expiresAt, now))}</span>
  <span class="bar" aria-hidden="true"><span class="fill" style:width="{share}%"></span></span>
</div>

<style>
  .countdown { display: flex; flex-direction: column; gap: 6px; font-size: var(--fs-sm); color: var(--text-2); }
  .bar { height: 4px; border-radius: var(--radius-pill); background: var(--surface-3); overflow: hidden; }
  .fill { display: block; height: 100%; background: var(--accent); transition: width 1s linear; }
  .low .fill { background: var(--danger); }
  .low .text { color: var(--danger-text); }
</style>
