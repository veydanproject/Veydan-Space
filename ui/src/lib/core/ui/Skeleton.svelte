<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The rows of a list that is still coming (design kit, skeleton): the shape
  of the real rows, never a word «Загрузка…» or a spinner over the page.
  Each row is an avatar (round, square or none), two lines of varying width
  and, if asked, a block at its end; they pulse 1.4 s, one after the other.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';

  interface Props {
    rows?: number;
    /** The row's height: 60 for most lists, 64 for the chats. */
    height?: number;
    avatar?: 'round' | 'square' | 'none';
    /** The avatar's side. */
    size?: number;
    /** A block at the end of the row (a time, a code, a button). */
    trailing?: boolean;
  }

  let { rows = 6, height = 60, avatar = 'round', size = 36, trailing = false }: Props = $props();

  const WIDTHS = [[62, 38], [48, 70], [70, 44], [54, 60], [40, 52], [66, 34]];
</script>

<div class="skeleton" role="status" aria-label={$t('loading')}>
  {#each { length: rows } as _, i (i)}
    <div class="row" style:height="{height}px" style:animation-delay="{i * 100}ms">
      {#if avatar !== 'none'}<span class="av" class:square={avatar === 'square'} style:width="{size}px" style:height="{size}px"></span>{/if}
      <span class="lines">
        <span class="line" style:width="{WIDTHS[i % WIDTHS.length][0]}%"></span>
        <span class="line sub" style:width="{WIDTHS[i % WIDTHS.length][1]}%"></span>
      </span>
      {#if trailing}<span class="end"></span>{/if}
    </div>
  {/each}
</div>

<style>
  .skeleton { display: flex; flex-direction: column; gap: 2px; }
  .row {
    display: flex; align-items: center; gap: var(--sp-3); padding: 0 var(--sp-3); box-sizing: border-box;
    animation: skeleton-pulse 1.4s ease-in-out infinite both;
  }
  .av, .line, .end { background: color-mix(in srgb, var(--text) 8%, transparent); border-radius: var(--radius-sm); flex-shrink: 0; }
  .av { border-radius: 50%; }
  .av.square { border-radius: var(--radius-md); }
  .lines { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-2); }
  .line { height: 12px; }
  .line.sub { height: 10px; }
  .end { width: 40px; height: 18px; }
  @keyframes skeleton-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.4; } }
  @media (prefers-reduced-motion: reduce) { .row { animation: none; } }
</style>
