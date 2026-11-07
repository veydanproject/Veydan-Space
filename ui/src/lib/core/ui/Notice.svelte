<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The plate of a page: a line that is not an action, a warning, a refusal, a
  confirmation. One shape everywhere, on a computer and on a phone: the tone
  colours the ground, the border and the icon (no tile), the text stays --text.
  Buttons go in `action`: secondary `btn btn-sm`, the main one last, red only
  for a dangerous one.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';

  type Tone = 'info' | 'ok' | 'warn' | 'danger';
  let { tone = 'info', icon, role, wait = false, children, action }: {
    tone?: Tone; icon?: string; role?: 'alert' | 'status'; wait?: boolean; children: Snippet; action?: Snippet;
  } = $props();

  const ICONS: Record<Tone, string> = { info: 'info', ok: 'check', warn: 'alert-triangle', danger: 'alert-triangle' };
</script>

<div class="notice {tone}" role={role ?? (tone === 'danger' ? 'alert' : wait ? 'status' : undefined)}>
  <span class="mark">{#if wait}<span class="spinner" aria-hidden="true"></span>{:else}<Icon name={icon ?? ICONS[tone]} size={16} />{/if}</span>
  <div class="text">{@render children()}</div>
  {#if action}<div class="action">{@render action()}</div>{/if}
</div>

<style>
  .notice {
    display: flex; align-items: flex-start; flex-wrap: wrap; gap: var(--sp-2) var(--sp-3);
    padding: var(--sp-3) var(--sp-4); border-radius: var(--radius-md);
    border: 1px solid var(--accent-tint-border); background: var(--accent-tint); color: var(--text);
    font-size: var(--fs-sm); line-height: var(--lh-base);
  }
  .mark { display: inline-flex; padding-top: 2px; color: var(--accent-text); flex-shrink: 0; }
  .text { flex: 1 1 12em; min-width: 0; overflow-wrap: anywhere; }
  .action { flex-shrink: 0; align-self: center; display: flex; flex-wrap: wrap; gap: var(--sp-2); margin-left: auto; }
  .ok { border-color: var(--success-border); background: var(--success-bg); }
  .ok .mark { color: var(--success-text); }
  .warn { border-color: var(--warn-border); background: var(--warn-bg); }
  .warn .mark { color: var(--warn-text); }
  .danger { border-color: var(--danger-border); background: var(--danger-bg); }
  .danger .mark { color: var(--danger-text); }
</style>
