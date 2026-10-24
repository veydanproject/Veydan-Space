<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- A line of the Tor section said as a plate with its mark: done, a warning, an error, a hint; the colour is never the only sign. -->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';

  type Tone = 'ok' | 'warn' | 'danger' | 'quiet';
  let { tone = 'quiet', icon, wait = false, children }: { tone?: Tone; icon?: string; wait?: boolean; children: Snippet } = $props();

  const ICONS: Record<Tone, string> = { ok: 'check', warn: 'alert-triangle', danger: 'alert-triangle', quiet: 'info' };
</script>

<div class="note {tone}" role={tone === 'danger' ? 'alert' : tone === 'quiet' && !wait ? undefined : 'status'}>
  {#if wait}<span class="spinner" aria-hidden="true"></span>{:else}<Icon name={icon ?? ICONS[tone]} size={18} />{/if}
  <span class="text">{@render children()}</span>
</div>

<style>
  .note {
    display: flex; align-items: flex-start; gap: var(--sp-3); padding: var(--sp-3) var(--sp-4);
    border-radius: var(--radius-md); font-size: var(--fs-base); line-height: 1.5; color: var(--text);
  }
  .note :global(svg), .spinner { flex-shrink: 0; margin-top: 2px; }
  .text { flex: 1; min-width: 0; overflow-wrap: anywhere; }
  .ok { background: var(--success-bg); }
  .ok :global(svg) { color: var(--success-text); }
  .warn { background: var(--warn-bg); }
  .warn :global(svg) { color: var(--warn-text); }
  .danger { background: var(--danger-bg); }
  .danger :global(svg) { color: var(--danger-text); }
  .quiet { background: var(--surface-2); color: var(--text-2); }
</style>
