<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- One conflict: local pane, remote pane, and the lines that will remain. -->
<script lang="ts">
  import type { MergeBlock } from '$lib/notes/types';
  import { shownLinePick, textLines, type LinePick, type MergeSelections } from '$lib/notes/merge-result';
  import { t } from '$lib/core/i18n';

  interface Props {
    blocks: MergeBlock[];
    index: number;
    choices: MergeSelections;
    picks: (LinePick | null)[];
    localLabel: string;
    remoteLabel: string;
    resultLabel?: string;
    resolved?: boolean;
    ontoggle: (side: 'ours' | 'theirs', line: number) => void;
  }

  let {
    blocks,
    index,
    choices,
    picks,
    localLabel,
    remoteLabel,
    resultLabel,
    resolved = false,
    ontoggle,
  }: Props = $props();

  const CONTEXT_LINES = 5;

  type GitRow = { n: number; text: string; change: boolean; pickAt?: number };

  function neighborLines(dir: -1 | 1): string[] {
    const other = blocks[index + dir];
    if (!other || other.kind !== 'normal') return [];
    const lines = textLines(other.text);
    return dir < 0 ? lines.slice(-CONTEXT_LINES) : lines.slice(0, CONTEXT_LINES);
  }

  function sideOrigin(side: 'ours' | 'theirs'): number {
    let n = 1;
    for (let i = 0; i < index; i++) {
      const b = blocks[i];
      const text = b.kind === 'normal' ? b.text : side === 'ours' ? b.ours : b.theirs;
      n += textLines(text).length;
    }
    return n;
  }

  /** One side: 5 lines before the change, the change, then 5 lines after. */
  function sideRows(side: 'ours' | 'theirs'): GitRow[] {
    const block = blocks[index];
    if (!block) return [];
    const body = textLines(side === 'ours' ? block.ours : block.theirs);
    const before = neighborLines(-1);
    const after = neighborLines(1);
    const origin = sideOrigin(side);
    const rows: GitRow[] = [];
    before.forEach((text, i) => rows.push({ n: origin - before.length + i, text, change: false }));
    body.forEach((text, i) => rows.push({ n: origin + i, text, change: true, pickAt: i }));
    after.forEach((text, i) => rows.push({ n: origin + body.length + i, text, change: false }));
    return rows;
  }

  /** Result window with the same context and only the lines still included. */
  function resultRows(pick: LinePick): GitRow[] {
    const block = blocks[index];
    if (!block) return [];
    const before = neighborLines(-1);
    const after = neighborLines(1);
    const picked = [
      ...textLines(block.ours).filter((_, i) => pick.ours[i]),
      ...textLines(block.theirs).filter((_, i) => pick.theirs[i]),
    ];
    let origin = 1;
    for (let i = 0; i < index; i++) {
      const b = blocks[i];
      if (!b) continue;
      if (b.kind === 'normal') origin += textLines(b.text).length;
      else {
        const p = shownLinePick(b, choices[i] ?? null, picks[i] ?? null);
        origin += textLines(b.ours).filter((_, j) => p.ours[j]).length;
        origin += textLines(b.theirs).filter((_, j) => p.theirs[j]).length;
      }
    }
    const rows: GitRow[] = [];
    before.forEach((text, i) => rows.push({ n: origin - before.length + i, text, change: false }));
    picked.forEach((text, i) => rows.push({ n: origin + i, text, change: true }));
    after.forEach((text, i) => rows.push({ n: origin + picked.length + i, text, change: false }));
    return rows;
  }

  function currentPick(): LinePick {
    const block = blocks[index];
    if (!block) return { ours: [], theirs: [] };
    return shownLinePick(block, choices[index] ?? null, picks[index] ?? null);
  }
</script>

{#snippet gitRows(rows: GitRow[], side: 'ours' | 'theirs' | null, pick: LinePick)}
  <ul class="git">
    {#each rows as row}
      <li class:change={row.change} class:on={side != null && row.pickAt != null && pick[side][row.pickAt]}>
        <span class="ln">{row.n}</span>
        {#if side != null && row.pickAt != null}
          <button type="button" class="line-plus" class:on={pick[side][row.pickAt]} onclick={() => ontoggle(side, row.pickAt ?? 0)}>+</button>
        {:else}
          <span class="gutter"></span>
        {/if}
        <span class="code">{row.text === '' ? ' ' : row.text}</span>
      </li>
    {:else}
      <li><span class="ln"></span><span class="gutter"></span><span class="code empty">{$t('notes_merge_empty')}</span></li>
    {/each}
  </ul>
{/snippet}

<div class="conflict-block" class:resolved>
  <div class="conflict-side side-current">
    <div class="side-label">{localLabel}</div>
    {@render gitRows(sideRows('ours'), 'ours', currentPick())}
  </div>
  <div class="conflict-side side-history">
    <div class="side-label">{remoteLabel}</div>
    {@render gitRows(sideRows('theirs'), 'theirs', currentPick())}
  </div>
  <div class="conflict-side side-result">
    <div class="side-label">{resultLabel ?? $t('notes_merge_result')}</div>
    {@render gitRows(resultRows(currentPick()), null, currentPick())}
  </div>
</div>

<style>
  .conflict-block {
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    overflow: hidden;
  }

  .conflict-block.resolved { border-color: var(--accent-border); }

  .conflict-side { padding: 0; }
  .side-history,
  .side-result { border-top: 1px solid var(--border); }

  .side-label {
    margin: 0;
    padding: 0.35rem 0.6rem;
    font-family: var(--font-mono);
    font-size: 12px;
    font-weight: 600;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
  }

  .side-current .side-label { color: var(--accent-text); }
  .side-history .side-label { color: var(--success-text); }
  .side-result .side-label { color: var(--text-2); }

  .git {
    list-style: none;
    margin: 0;
    padding: 0.15rem 0;
    max-height: 13rem;
    overflow: auto;
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 1.35;
  }

  .git li {
    display: grid;
    grid-template-columns: 2.4rem 1.75rem minmax(0, 1fr);
    column-gap: 0.4rem;
    align-items: center;
    min-height: 1.75rem;
    padding: 0 0.45rem;
    white-space: pre;
  }

  .side-current .git li.change { background: var(--accent-tint); }
  .side-history .git li.change { background: var(--success-bg); }
  .side-result .git li.change { background: var(--surface-2); }
  .git li.on { box-shadow: inset 2px 0 0 var(--accent); }

  .ln {
    text-align: right;
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
    user-select: none;
  }

  .gutter { width: 1.75rem; }

  .code { color: var(--text); }
  .git li:not(.change) .code { color: var(--text-3); }
  .code.empty { color: var(--text-3); }

  .line-plus {
    width: 1.75rem;
    height: 1.75rem;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 3px;
    background: var(--surface);
    color: var(--text-2);
    cursor: pointer;
    font-family: var(--font-mono);
    font-size: 14px;
    line-height: 1;
  }

  .line-plus.on {
    background: var(--accent-bg);
    color: var(--accent-text);
    border-color: var(--accent-border);
  }
</style>
