<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  One file in a message: its mark (the action, a ring round it while it moves),
  its name and what is known of it on one line under the name. The bubble's
  time stands at the end of that line (`--file-meta` keeps the room for it).
-->
<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    name: string;
    title?: string;
    sub?: string;
    failed?: boolean;
    mark: Snippet;
    right?: Snippet;
  }
  let { name, title, sub = '', failed = false, mark, right }: Props = $props();
</script>

<div class="file-line" class:failed>
  <span class="mark">{@render mark()}</span>
  <span class="name" title={title ?? name}>{name}</span>
  {#if right}<span class="right">{@render right()}</span>{/if}
  {#if sub}<span class="sub">{sub}</span>{/if}
</div>

<style>
  .file-line {
    display: grid; grid-template-columns: 44px minmax(0, 1fr) auto; grid-template-areas: "mark name right" "mark sub sub";
    column-gap: var(--sp-3); align-items: center; min-height: 44px; min-width: 0;
  }
  .mark { grid-area: mark; display: inline-flex; width: 44px; height: 44px; position: relative; }
  .name { grid-area: name; min-width: 0; align-self: end; font-size: var(--fs-base); font-weight: var(--fw-bold); line-height: var(--lh-base); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .right { grid-area: right; display: inline-flex; align-self: start; }
  .sub { grid-area: sub; min-width: 0; align-self: start; padding-right: var(--file-meta, 0); font-size: var(--fs-sm); line-height: var(--lh-base); color: var(--text-3); overflow-wrap: anywhere; font-variant-numeric: tabular-nums; }
  .failed .sub { color: var(--danger-text); }
</style>
