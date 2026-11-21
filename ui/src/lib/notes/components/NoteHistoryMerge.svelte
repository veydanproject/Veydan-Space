<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import type { MergeBlock, MergeResult } from '$lib/notes/types';
  import Pill from '$lib/core/ui/Pill.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { t } from '$lib/core/i18n';
  import MergeHunk from './MergeHunk.svelte';
  import {
    buildResolvedContent,
    emptySelections,
    nextLineToggle,
    selectAllChoice,
    shownLinePick,
    unresolvedCount as countUnresolved,
    type LinePick,
    type MergeChoice,
    type MergeSelections,
  } from '$lib/notes/merge-result';

  interface Props {
    /** Fetches the merge; history merge and sync conflicts share this UI */
    load: () => Promise<MergeResult>;
    /** Side titles. Device names are appended when set. */
    localTitle?: string;
    remoteTitle?: string;
    localName?: string;
    remoteName?: string;
    oursAction?: string;
    bothAction?: string;
    theirsAction?: string;
    onresolved: (content: string) => void;
    oncancel: () => void;
  }

  let {
    load,
    localTitle,
    remoteTitle,
    localName = '',
    remoteName = '',
    oursAction,
    bothAction,
    theirsAction,
    onresolved,
    oncancel,
  }: Props = $props();

  // Without a title of their own the sides are the current text and a version from history
  const localHead = $derived(localTitle ?? $t('notes_merge_current'));
  const remoteHead = $derived(remoteTitle ?? $t('notes_merge_history'));

  let blocks = $state<MergeBlock[]>([]);
  let choices = $state<MergeSelections>([]);
  /** Set when the user toggles lines; null means the whole-side choice applies. */
  let picks = $state<(LinePick | null)[]>([]);
  let loading = $state(true);
  let error = $state('');
  let hasConflicts = $state(false);
  let scrollEl = $state<HTMLDivElement | null>(null);

  $effect(() => {
    loadMerge();
  });

  // Unchanged text can fill the window; keep the conflict, and its line buttons, in view.
  $effect(() => {
    if (loading || !hasConflicts || !scrollEl) return;
    scrollEl.querySelector('.conflict-block')?.scrollIntoView({ block: 'start' });
  });

  async function loadMerge() {
    loading = true;
    error = '';
    try {
      const result = await load();
      blocks = result.blocks;
      choices = emptySelections(blocks);
      picks = blocks.map(() => null);
      // Only blocks the user can act on count; never claim "all resolved" over plain text
      hasConflicts = blocks.some((b) => b.kind === 'conflict');
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  function sideHead(title: string, name: string): string {
    const n = name.trim();
    return n ? `${title} · ${n}` : title;
  }

  /** Picks one whole-block choice for every conflict. */
  function chooseAll(choice: MergeChoice) {
    choices = selectAllChoice(blocks, choice);
    picks = blocks.map(() => null);
  }

  function allChosen(choice: MergeChoice): boolean {
    return blocks.some((b) => b.kind === 'conflict')
      && blocks.every((b, i) => b.kind !== 'conflict' || (choices[i] === choice && !picks[i]));
  }

  /** Toggles one line. A pattern that matches a whole side collapses back to that choice. */
  function toggleLine(index: number, side: 'ours' | 'theirs', line: number) {
    const block = blocks[index];
    if (!block) return;
    const next = nextLineToggle(block, shownLinePick(block, choices[index] ?? null, picks[index] ?? null), side, line);
    if (!next) return;
    choices[index] = next.choice;
    picks[index] = next.pick;
  }

  function blockResolved(index: number): boolean {
    return choices[index] != null || picks[index] != null;
  }

  const unresolvedCount = $derived(countUnresolved(blocks, choices, picks));
  const allResolved = $derived(unresolvedCount === 0);

  function apply() {
    onresolved(buildResolvedContent(blocks, choices, picks));
  }
</script>

<Dialog open={true} title={$t('notes_merge_title')} width="min(900px, 92vw)" onclose={oncancel}>
  {#if loading}
    <div class="loading loading-center">
      <Icon name="loader" size={18} />
      <span>{$t('notes_merge_running')}</span>
    </div>
  {:else if error}
    <div class="error-bar">{error}</div>
  {:else if !hasConflicts}
    <div class="no-conflicts">
      <Icon name="check" size={20} />
      <p>{$t('notes_merge_clean')}</p>
      <span>{$t('notes_merge_clean_hint')}</span>
    </div>
  {:else}
    <div class="conflicts-summary">
      <div class="summary-top">
        {#if unresolvedCount > 0}
          <Pill tone="warn">
            <Icon name="alert-triangle" size={12} />
            {$t('notes_merge_unresolved', { n: String(unresolvedCount) })}
          </Pill>
        {:else}
          <Pill tone="ok">
            <Icon name="check" size={12} />
            {$t('notes_merge_resolved')}
          </Pill>
        {/if}
      </div>
      <div class="summary-actions">
        <button type="button" class="btn btn-ghost btn-sm" class:active={allChosen('ours')} onclick={() => chooseAll('ours')}>
          {oursAction ?? $t('notes_merge_keep_current')}
        </button>
        <button type="button" class="btn btn-ghost btn-sm" class:active={allChosen('both')} onclick={() => chooseAll('both')}>
          {bothAction ?? $t('notes_merge_keep_both')}
        </button>
        <button type="button" class="btn btn-ghost btn-sm" class:active={allChosen('theirs')} onclick={() => chooseAll('theirs')}>
          {theirsAction ?? $t('notes_merge_keep_history')}
        </button>
      </div>
    </div>

    <div class="blocks" bind:this={scrollEl}>
      {#each blocks as block, i}
        {#if block.kind === 'conflict'}
          <MergeHunk
            {blocks}
            index={i}
            {choices}
            {picks}
            localLabel={sideHead(localHead, localName)}
            remoteLabel={sideHead(remoteHead, remoteName)}
            resolved={blockResolved(i)}
            ontoggle={(side, line) => toggleLine(i, side, line)}
          />
        {/if}
      {/each}
    </div>
  {/if}

  {#snippet footer()}
    {#if !loading}
      <button class="btn btn-ghost" onclick={oncancel}>{$t('common_cancel')}</button>
      {#if !error}
        <button class="btn btn-primary" onclick={apply} disabled={!allResolved}>
          {$t('notes_merge_apply')}
        </button>
      {/if}
    {/if}
  {/snippet}
</Dialog>

<style>
  /* .loading covers the layout; keep only the flex-fill delta for the window */
  .loading-center { flex: 1; }

  .error-bar {
    background: var(--danger-bg);
    color: var(--danger-text);
    padding: var(--sp-2) var(--sp-4);
    border-radius: var(--radius-md);
    font-size: var(--fs-sm);
  }

  .no-conflicts {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--sp-2);
    color: var(--success-text);
    text-align: center;
    padding: var(--sp-8);
  }

  .no-conflicts p { margin: 0; font-size: var(--fs-md); font-weight: var(--fw-medium); }
  .no-conflicts span { font-size: var(--fs-sm); color: var(--text-2); }

  .conflicts-summary {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    padding-bottom: var(--sp-3);
    margin-bottom: var(--sp-3);
    border-bottom: 1px solid var(--border);
    gap: var(--sp-2);
  }

  .summary-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-2);
  }

  .summary-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-2);
  }

  .summary-actions .btn { flex: 1 1 8rem; }

  .summary-actions .btn.active {
    background: var(--accent-bg);
    color: var(--accent-text);
    border-color: var(--accent-border);
  }

  .blocks {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    font-size: var(--fs-sm);
  }
</style>
