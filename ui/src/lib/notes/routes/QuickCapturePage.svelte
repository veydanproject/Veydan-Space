<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/notes/api';
  import { notesStore } from '$lib/notes/store/notes.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';

  let text = $state('');
  let folderId = $state('');
  let tags = $state('');
  let saving = $state(false);
  let savedFlash = $state(false);
  let textarea = $state<HTMLTextAreaElement | null>(null);

  const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

  onMount(() => {
    void notesStore.refreshFolders();
    textarea?.focus();
    if (!isTauri) return;
    // Re-focus when the window is shown again via shortcut/tray
    let unlisten = () => {};
    void import('@tauri-apps/api/event').then(({ listen }) =>
      listen('quick-capture://shown', () => textarea?.focus()).then((u) => (unlisten = u))
    );
    return () => unlisten();
  });

  async function closeWindow() {
    if (!isTauri) return;
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    await getCurrentWindow().hide();
  }

  async function pasteClipboard() {
    try {
      const clip = await navigator.clipboard.readText();
      if (clip) text = text ? `${text}\n${clip}` : clip;
      textarea?.focus();
    } catch {}
  }

  /** First non-empty line is the title, the rest is the body */
  function split(raw: string): { title: string; content: string } {
    const lines = raw.replace(/\r/g, '').split('\n');
    const idx = lines.findIndex((l) => l.trim());
    if (idx < 0) return { title: '', content: '' };
    const title = lines[idx].replace(/^#+\s*/, '').trim().slice(0, 120);
    const content = lines.slice(idx + 1).join('\n').trim();
    return { title, content };
  }

  async function save() {
    const { title, content } = split(text);
    if (!title || saving) return;
    saving = true;
    try {
      const note = await api.notes.create({
        title,
        content,
        bindings: [],
        tag_names: tags.split(',').map((s) => s.trim()).filter(Boolean),
      });
      if (folderId) await api.notes.noteAddFolder(note.id, folderId);
      text = '';
      tags = '';
      savedFlash = true;
      setTimeout(() => (savedFlash = false), 1200);
      await closeWindow();
    } finally {
      saving = false;
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') { e.preventDefault(); void closeWindow(); }
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); void save(); }
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div class="quick">
  <textarea
    bind:this={textarea}
    bind:value={text}
    class="text"
    placeholder={$t('quick_capture_placeholder')}
    spellcheck="false"
  ></textarea>

  <div class="meta">
    <select class="field" bind:value={folderId} title={$t('quick_capture_folder')}>
      <option value="">{$t('quick_capture_no_folder')}</option>
      {#each notesStore.folders as f (f.id)}
        <option value={f.id}>{f.name}</option>
      {/each}
    </select>
    <input class="field" type="text" bind:value={tags} placeholder={$t('quick_capture_tags')} />
  </div>

  <div class="footer">
    <span class="hint">{savedFlash ? $t('quick_capture_saved') : $t('quick_capture_hint')}</span>
    <div class="actions">
      <button class="btn btn-ghost btn-sm" onclick={pasteClipboard} title={$t('quick_capture_paste')}>
        <Icon name="copy" size={13} />
      </button>
      <button class="btn btn-primary btn-sm" onclick={save} disabled={!split(text).title || saving}>
        {$t('quick_capture_save')}
      </button>
    </div>
  </div>
</div>

<style>
  .quick {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    height: 100%;
    padding: 0.75rem;
    box-sizing: border-box;
  }
  .text {
    flex: 1;
    min-height: 120px;
    resize: none;
    padding: 0.6rem;
    font: inherit;
    font-size: var(--fs-sm);
    line-height: 1.5;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg-2);
    color: var(--text);
  }
  .text:focus { outline: none; border-color: var(--accent); }
  .meta { display: grid; grid-template-columns: 1fr 1.4fr; gap: 0.4rem; }
  .field {
    min-width: 0;
    padding: 0.35rem 0.5rem;
    font-size: var(--fs-xs);
    border: 1px solid var(--border);
    border-radius: 6px;
    /* The colour alone: the folder select keeps the global chevron. */
    background-color: var(--bg-2);
    color: var(--text);
  }
  select.field { padding-right: 1.6rem; background-position: right 0.4rem center; }
  .footer { display: flex; align-items: center; justify-content: space-between; gap: 0.5rem; }
  .hint { font-size: var(--fs-2xs); color: var(--text-3); }
  .actions { display: flex; gap: 0.4rem; }
</style>
