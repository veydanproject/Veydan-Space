<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A word in a pill, one for the whole interface, in two kinds:
  - `badge` (the default) — a mark beside a name, in a head or a cell: a
    frame, `--fs-xs`; its look is `.badge` of base.css. `onclick` makes it a
    button (a proxy in a table that opens it).
  - `state` — the state at the end of a row or a head: 22, no frame.
  `tone`: `ok` on its way, `warn` waiting, `bad` gone wrong, `accent` what
  the row is about, `neutral` not known yet or only a word.
-->
<script module lang="ts">
  export type PillTone = 'ok' | 'warn' | 'bad' | 'accent' | 'neutral';
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLAttributes } from 'svelte/elements';

  interface Props extends HTMLAttributes<HTMLElement> {
    /** The word; or `children` for a word with an icon. */
    text?: string;
    tone?: PillTone;
    kind?: 'badge' | 'state';
    onclick?: (e: MouseEvent) => void;
    children?: Snippet;
  }
  let { text, tone = 'neutral', kind = 'badge', onclick, children, class: cls = '', ...rest }: Props = $props();

  const BADGE: Record<PillTone, string> = { ok: 'badge-ok', warn: 'badge-warn', bad: 'badge-danger', accent: 'badge-accent', neutral: '' };
  const classes = $derived(kind === 'badge' ? `badge ${BADGE[tone]} ${cls}` : `state ${tone} ${cls}`);
</script>

{#snippet body()}{#if children}{@render children()}{:else}{text}{/if}{/snippet}

{#if onclick}
  <button type="button" {...rest} class={classes} {onclick}>{@render body()}</button>
{:else}
  <span {...rest} class={classes}>{@render body()}</span>
{/if}

<style>
  .state {
    flex: none; display: inline-flex; align-items: center; height: 22px; padding: 0 var(--sp-2); border-radius: var(--radius-pill);
    font-size: var(--fs-xs); font-weight: var(--fw-semibold); white-space: nowrap;
    background: var(--success-bg); color: var(--success-text, var(--success));
  }
  .state.warn { background: var(--warn-bg); color: var(--warn-text); }
  .state.bad { background: var(--danger-bg); color: var(--danger-text); }
  .state.accent { background: var(--accent-tint); color: var(--accent-text-2); }
  .state.neutral { background: var(--surface-2); color: var(--text-2); }
</style>
