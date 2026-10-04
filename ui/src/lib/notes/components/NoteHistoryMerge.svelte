<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import type { MergeBlock, MergeResult } from '$lib/notes/types';
  import Icon from '$lib/core/Icon.svelte';
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

  // Unchanged text can fill the panel; keep the conflict, and its line buttons, in view.
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

<div class="merge-overlay">
  <div class="merge-panel">
    <div class="merge-header">
      <span class="merge-title">
        <Icon name="git-merge" size={14} />
        {$t('notes_merge_title')}
      </span>
      <button class="btn-cancel-header" onclick={oncancel} title={$t('notes_btn_cancel')}>
        <Icon name="x" size={14} />
      </button>
    </div>

    {#if loading}
      <div class="loading loading-center">
        <Icon name="loader" size={18} />
        <span>{$t('notes_merge_running')}</span>
      </div>
    {:else if error}
      <div class="error-bar">{error}</div>
      <div class="footer">
        <button class="btn btn-ghost btn-sm" onclick={oncancel}>{$t('notes_btn_cancel')}</button>
      </div>
    {:else if !hasConflicts}
      <div class="no-conflicts">
        <Icon name="check-circle" size={20} />
        <p>{$t('notes_merge_clean')}</p>
        <span>{$t('notes_merge_clean_hint')}</span>
      </div>
      <div class="footer">
        <button class="btn btn-ghost btn-sm" onclick={oncancel}>{$t('notes_btn_cancel')}</button>
        <button class="btn btn-primary btn-sm" onclick={apply}>{$t('notes_merge_apply')}</button>
      </div>
    {:else}
      <div class="conflicts-summary">
        <div class="summary-top">
          {#if unresolvedCount > 0}
            <span class="badge badge-warn">
              <Icon name="alert-triangle" size={12} />
              {$t('notes_merge_unresolved', { n: String(unresolvedCount) })}
            </span>
          {:else}
            <span class="badge badge-ok">
              <Icon name="check-circle" size={12} />
              {$t('notes_merge_resolved')}
            </span>
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

      <div class="blocks-scroll" bind:this={scrollEl}>
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

      <div class="footer">
        <button class="btn btn-ghost btn-sm" onclick={oncancel}>{$t('notes_btn_cancel')}</button>
        <button class="btn btn-primary btn-sm" onclick={apply} disabled={!allResolved}>
          {$t('notes_merge_apply')}
        </button>
      </div>
    {/if}
  </div>
</div>

<style>
  .merge-overlay {
    position: absolute;
    inset: 0;
    background: var(--backdrop);
    display: flex;
    align-items: stretch;
    justify-content: flex-end;
    z-index: var(--z-panel);
  }

  .merge-panel {
    width: 100%;
    max-width: 640px;
    background: var(--surface);
    border-left: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    font-size: var(--fs-sm);
  }

  .merge-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.6rem var(--sp-4);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    background: var(--surface-2);
  }

  .merge-title {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    font-weight: 600;
    font-size: var(--fs-base);
    color: var(--text);
  }

  .btn-cancel-header {
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-3);
    padding: 0.2rem;
    border-radius: var(--radius-sm);
    display: flex;
    align-items: center;
    transition: color 0.15s;
  }

  .btn-cancel-header:hover { color: var(--text); }

  /* .loading covers the layout; keep only the flex-fill delta for the overlay */
  .loading-center { flex: 1; }

  .error-bar {
    background: var(--danger-bg);
    color: var(--danger-text);
    padding: var(--sp-2) var(--sp-4);
    font-size: var(--fs-sm);
  }

  .no-conflicts {
    flex: 1;
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
    padding: 0.4rem var(--sp-4);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
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

  .blocks-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: var(--sp-2);
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .footer {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--sp-2);
    padding: 0.6rem var(--sp-4);
    border-top: 1px solid var(--border);
    flex-shrink: 0;
    background: var(--surface-drawer-footer);
  }

</style>
