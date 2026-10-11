<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The end of a flow, or a stop in it: a mark, what happened, what to do next. -->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';

  type Tone = 'ok' | 'warn' | 'danger' | 'info';
  let { tone = 'ok', icon, title, children, actions }: { tone?: Tone; icon?: string; title: string; children?: Snippet; actions?: Snippet } = $props();

  const ICONS: Record<Tone, string> = { ok: 'check', warn: 'alert-triangle', danger: 'alert-triangle', info: 'clock' };
</script>

<div class="state {tone}">
  <span class="mark"><Icon name={icon ?? ICONS[tone]} size={26} /></span>
  <h2>{title}</h2>
  {#if children}<div class="text">{@render children()}</div>{/if}
  {#if actions}<div class="actions">{@render actions()}</div>{/if}
</div>

<style>
  .state { display: flex; flex-direction: column; align-items: center; gap: var(--sp-3); text-align: center; padding: var(--sp-4) 0 var(--sp-2); }
  .mark {
    display: inline-flex; align-items: center; justify-content: center; width: 56px; height: 56px; border-radius: 50%;
    background: var(--success-bg); color: var(--success-text); border: 1px solid var(--success-border);
  }
  .warn .mark { background: var(--warn-bg); color: var(--warn-text); border-color: var(--warn-border); }
  .danger .mark { background: var(--danger-bg); color: var(--danger-text); border-color: var(--danger-border); }
  .info .mark { background: var(--accent-tint); color: var(--accent-text); border-color: var(--accent-tint-border); }
  h2 { margin: 0; font-size: var(--fs-lg); font-weight: var(--fw-bold); color: var(--text); }
  .text { color: var(--text-2); font-size: var(--fs-base); line-height: 1.5; max-width: 420px; }
  .text :global(p) { margin: 0; }
  .actions { align-self: stretch; margin-top: var(--sp-2); display: flex; flex-direction: column; gap: var(--sp-3); }
  /* Under a centred mark the buttons are centred too. */
  .actions :global(.actions) { justify-content: center; }
</style>
