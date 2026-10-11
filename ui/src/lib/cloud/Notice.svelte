<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- A line of the page that is not an action: a warning, a refusal, a confirmation. One shape for all of them, the tone by colour and icon. -->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';

  type Tone = 'info' | 'ok' | 'warn' | 'danger';
  let { tone = 'info', icon, role, children, action }: { tone?: Tone; icon?: string; role?: 'alert' | 'status'; children: Snippet; action?: Snippet } = $props();

  const ICONS: Record<Tone, string> = { info: 'info', ok: 'check-circle', warn: 'alert-triangle', danger: 'alert-triangle' };
</script>

<div class="notice {tone}" role={role ?? (tone === 'danger' ? 'alert' : undefined)}>
  <span class="mark"><Icon name={icon ?? ICONS[tone]} size={16} /></span>
  <div class="text">{@render children()}</div>
  {#if action}<div class="action">{@render action()}</div>{/if}
</div>

<style>
  .notice {
    display: flex; align-items: flex-start; gap: var(--sp-3);
    padding: var(--sp-3) var(--sp-4); border-radius: var(--radius-md);
    border: 1px solid var(--accent-tint-border); background: var(--accent-tint); color: var(--text-body);
    font-size: var(--fs-sm); line-height: 1.5;
  }
  .mark { display: inline-flex; padding-top: 2px; color: var(--accent-text); flex-shrink: 0; }
  .text { flex: 1; min-width: 0; }
  .action { flex-shrink: 0; align-self: center; }
  .ok { border-color: var(--success-border); background: var(--success-bg); }
  .ok .mark { color: var(--success-text); }
  .warn { border-color: var(--warn-border); background: var(--warn-bg); }
  .warn .mark { color: var(--warn-text); }
  .danger { border-color: var(--danger-border); background: var(--danger-bg); color: var(--danger-text); }
  .danger .mark { color: var(--danger-text); }
</style>
