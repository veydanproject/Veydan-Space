<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Sync conflict resolver. Shows the incoming lines before a side is chosen.
     Works on one conflictGet snapshot; a changed token reloads it. -->
<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { api, formatError, onSyncStatus, type ConflictView, type MergeBlock } from '$lib/notes/mobile/api';
  import { t } from '$lib/core/mobile/i18n';
  import { hasErrorCode } from '$lib/core/utils';
  import MergeHunk from '$lib/notes/components/MergeHunk.svelte';
  import {
    buildResolvedContent,
    emptySelections,
    isComplete,
    nextLineToggle,
    selectAllChoice,
    shownLinePick,
    unresolvedCount,
    type LinePick,
    type MergeChoice,
    type MergeSelections,
  } from '$lib/notes/merge-result';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';

  interface Props {
    open: boolean;
    noteId: string;
    onclose: () => void;
    /** Reloads note and sync status; the sheet closes only after it succeeds. */
    onresolved: () => Promise<void>;
  }

  let { open, noteId, onclose, onresolved }: Props = $props();

  let view = $state<ConflictView | null>(null);
  let blocks = $state<MergeBlock[]>([]);
  let choices = $state<MergeSelections>([]);
  let picks = $state<(LinePick | null)[]>([]);
  let loading = $state(false);
  let busy = $state(false);
  let error = $state('');
  let notice = $state('');

  const left = $derived(unresolvedCount(blocks, choices, picks));
  const complete = $derived(isComplete(blocks, choices, picks));

  $effect(() => {
    if (!open) return;
    view = null;
    notice = '';
    void load();
  });

  onMount(() => {
    const un = onSyncStatus(() => {
      if (open && !busy) void load();
    });
    return () => un.then((f) => f());
  });

  /** Fetches the snapshot; a new token resets the choices and tells the user. */
  async function load() {
    loading = true;
    error = '';
    try {
      const next = await api.sync.conflictGet(noteId);
      if (view?.token === next.token) return;
      if (view) notice = $t('note_sync_conflict_changed');
      view = next;
      blocks = next.merge.blocks;
      choices = emptySelections(blocks);
      picks = blocks.map(() => null);
    } catch (e) {
      // Resolved elsewhere: nothing left to choose.
      if (view && hasErrorCode(e, 'not_found')) return finish();
      error = formatError(e);
    } finally {
      loading = false;
    }
  }

  async function finish() {
    await onresolved();
    onclose();
  }

  async function resolve(content: string) {
    if (!view) return;
    busy = true;
    error = '';
    try {
      await api.sync.conflictResolve(noteId, view.token, content);
      await finish();
    } catch (e) {
      if (hasErrorCode(e, 'conflict_changed')) await load();
      else error = formatError(e);
    } finally {
      busy = false;
    }
  }

  const applyReview = () => resolve(buildResolvedContent(blocks, choices, picks));

  function chooseAll(choice: MergeChoice) {
    choices = selectAllChoice(blocks, choice);
    picks = blocks.map(() => null);
  }

  function allChosen(choice: MergeChoice): boolean {
    return blocks.some((b) => b.kind === 'conflict')
      && blocks.every((b, i) => b.kind !== 'conflict' || (choices[i] === choice && !picks[i]));
  }

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

  function sideHead(title: string, name: string): string {
    const n = name.trim();
    return n ? `${title} · ${n}` : title;
  }
</script>

<BottomSheet {open} title={$t('note_sync_conflict_title')} {onclose}>
  {#if error}<div class="m-error">{error}</div>{/if}
  {#if notice}<div class="notice"><Icon name="alert-triangle" size={14} /> {notice}</div>{/if}

  {#if loading && !view}
    <p class="empty"><Icon name="loader" size={16} /></p>
  {:else if !view}
    <p class="empty">{$t('common_nothing_found')}</p>
  {:else}
    <div class="bulk">
      <button type="button" class="m-chip" class:active={allChosen('ours')} disabled={busy} onclick={() => chooseAll('ours')}>{$t('note_sync_conflict_accept_local')}</button>
      <button type="button" class="m-chip" class:active={allChosen('both')} disabled={busy} onclick={() => chooseAll('both')}>{$t('note_sync_conflict_accept_both')}</button>
      <button type="button" class="m-chip" class:active={allChosen('theirs')} disabled={busy} onclick={() => chooseAll('theirs')}>{$t('note_sync_conflict_accept_remote')}</button>
      {#if left > 0}
        <span class="left">{$t('note_sync_conflict_unresolved', { n: String(left) })}</span>
      {/if}
    </div>

    <div class="blocks">
      {#each blocks as block, i (i)}
        {#if block.kind === 'conflict'}
          <MergeHunk
            {blocks}
            index={i}
            {choices}
            {picks}
            localLabel={sideHead($t('note_sync_conflict_local'), view.local_device || $t('note_sync_conflict_this_computer'))}
            remoteLabel={sideHead($t('note_sync_conflict_remote_side'), view.remote_device)}
            resultLabel={$t('note_sync_conflict_result')}
            resolved={blockResolved(i)}
            ontoggle={(side, line) => toggleLine(i, side, line)}
          />
        {/if}
      {/each}
    </div>

    <button type="button" class="btn btn-primary" disabled={busy || !complete} onclick={applyReview}>
      <Icon name="check" size={16} /> {$t('note_sync_conflict_apply')}
    </button>
  {/if}
</BottomSheet>

<style>
  .empty { color: var(--text-3); text-align: center; padding: var(--sp-6) 0; }
  .notice {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    color: var(--warn-text, var(--text-2));
    font-size: var(--fs-sm);
  }
  .bulk { display: flex; align-items: center; gap: var(--sp-2); flex-wrap: wrap; }
  .left { margin-left: auto; font-size: var(--fs-xs); color: var(--text-3); }
  .blocks {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }
  .btn { display: inline-flex; align-items: center; justify-content: center; gap: 8px; }
</style>
