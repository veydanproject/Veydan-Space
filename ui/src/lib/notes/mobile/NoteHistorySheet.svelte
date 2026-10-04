<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Version list of a note; a tapped version is shown rendered and can be restored. -->
<script lang="ts">
  import { ask } from '$lib/core/ui/confirm.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { api, formatError, type Note, type NoteHistoryEntry } from '$lib/notes/mobile/api';
  import { locale, t } from '$lib/core/mobile/i18n';
  import { AttachmentUrls, renderMarkdown } from '$lib/notes/mobile/markdown';
  import { fmtDateTime } from '$lib/notes/mobile/notes-editor';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';

  interface Props {
    open: boolean;
    noteId: string;
    urls: AttachmentUrls;
    onclose: () => void;
    onrestored: (note: Note) => void;
  }

  let { open, noteId, urls, onclose, onrestored }: Props = $props();

  let entries = $state<NoteHistoryEntry[]>([]);
  let selected = $state<NoteHistoryEntry | null>(null);
  let html = $state('');
  let loading = $state(false);
  let error = $state('');

  $effect(() => {
    if (!open) return;
    selected = null;
    void loadList();
  });

  async function loadList() {
    loading = true;
    error = '';
    try {
      entries = await api.notes.historyList(noteId);
    } catch (e) {
      error = formatError(e);
    } finally {
      loading = false;
    }
  }

  async function show(entry: NoteHistoryEntry) {
    loading = true;
    error = '';
    try {
      const full = await api.notes.historyGet(entry.id);
      html = await renderMarkdown(full.content ?? '', noteId, urls);
      selected = full;
    } catch (e) {
      error = formatError(e);
    } finally {
      loading = false;
    }
  }

  async function restore() {
    if (!selected) return;
    if (!(await ask({ title: $t('notes_history_restore_confirm', { n: String(selected.revision) }), variant: 'primary' }))) return;
    loading = true;
    try {
      onrestored(await api.notes.historyRestore(noteId, selected.id));
      onclose();
    } catch (e) {
      error = formatError(e);
    } finally {
      loading = false;
    }
  }

  const typeLabel = (type: string) =>
    type === 'sync' ? $t('notes_history_type_sync') : type === 'restore' ? $t('notes_history_type_restore') : $t('notes_history_type_autosave');
</script>

<BottomSheet
  {open}
  title={selected ? $t('notes_history_version', { n: String(selected.revision) }) : $t('notes_history')}
  {onclose}
  onback={selected ? () => (selected = null) : undefined}
>
  {#if error}<div class="m-error">{error}</div>{/if}

  {#if selected}
    <div class="meta">
      <span class="m-chip small">{typeLabel(selected.version_type)}</span>
      {#if selected.device}<span class="m-chip small">{selected.device}</span>{/if}
      <span class="when">{fmtDateTime(selected.created_at, $locale)}</span>
    </div>
    {#if selected.title}<h3 class="vtitle">{selected.title}</h3>{/if}
    <!-- eslint-disable-next-line svelte/no-at-html-tags -- sanitized by DOMPurify -->
    <div class="preview md">{@html html}</div>
    <button type="button" class="btn btn-primary" disabled={loading} onclick={restore}>
      <Icon name="rotate-ccw" size={16} /> {$t('notes_history_restore')}
    </button>
  {:else if entries.length === 0 && !loading}
    <p class="empty">{$t('notes_history_empty')}</p>
  {:else}
    <div class="m-list">
      {#each entries as e (e.id)}
        <button type="button" class="m-row" onclick={() => show(e)}>
          <span class="rev">{e.revision}</span>
          <span class="m-row-label">
            <span class="row-title">{e.title || $t('notes_untitled')}</span>
            <span class="row-sub">
              {typeLabel(e.version_type)}{#if e.device} · {e.device}{/if} · {fmtDateTime(e.created_at, $locale)}
            </span>
          </span>
          <Icon name="chevron-right" size={16} />
        </button>
      {/each}
    </div>
  {/if}
</BottomSheet>

<style>
  .empty { color: var(--text-3); text-align: center; padding: var(--sp-6) 0; }
  .rev {
    min-width: 28px;
    text-align: center;
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    color: var(--text-3);
  }
  .m-row-label { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .row-title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .row-sub { font-size: var(--fs-xs); color: var(--text-3); }
  .meta { display: flex; align-items: center; gap: var(--sp-2); }
  .when { font-size: var(--fs-xs); color: var(--text-3); }
  .vtitle { margin: 0; font-size: var(--fs-lg); }
  .preview {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: var(--sp-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-md, 8px);
    font-size: 14px;
    line-height: 1.55;
    word-break: break-word;
    user-select: text;
    -webkit-user-select: text;
  }
  .preview :global(img) { max-width: 100%; border-radius: var(--radius-md, 8px); }
  .preview :global(pre) { overflow-x: auto; padding: var(--sp-2); background: var(--surface-2); border-radius: 6px; font-size: 12px; }
  .preview :global(code) { font-family: var(--font-mono); }
  .preview :global(a) { color: var(--accent-text); }
  .preview :global(h1), .preview :global(h2), .preview :global(h3) { margin: var(--sp-2) 0 var(--sp-1); }
  .preview :global(table) { border-collapse: collapse; }
  .preview :global(td), .preview :global(th) { border: 1px solid var(--border); padding: 3px 6px; }
  .btn { display: inline-flex; align-items: center; justify-content: center; gap: 8px; }
</style>
