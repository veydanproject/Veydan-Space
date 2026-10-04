<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { Editor } from '@tiptap/core';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { api, downloadNoteAttachment } from '$lib/notes/api';
  import { pasteHasHiddenFiles } from '$lib/notes/files';
  import { noteExtensions, unclosedMarkAt, TPL_CLOSE, TPL_OPEN, WIKI_CLOSE, WIKI_OPEN } from '$lib/notes/tiptap-ext';
  import PlaceholderPicker from './PlaceholderPicker.svelte';
  import type { EditAction } from '$lib/notes/markdown-edit';
  import type { NoteListItem } from '$lib/notes/types';
  import Icon from '$lib/core/Icon.svelte';
  import WikiLinkPicker from './WikiLinkPicker.svelte';
  import { t } from '$lib/core/i18n';
  import { stockHistoryChord, stockMarkChord, takeHistory } from '$lib/core/keybindings';

  interface Props {
    content: string;
    /** Absolute dir the relative attachment links resolve against */
    baseDir?: string;
    noteId?: string;
    readonly?: boolean;
    placeholder?: string;
    notes: NoteListItem[];
    excludeId?: string | null;
    onchange: (md: string) => void;
    onwikilink: (target: string) => void;
    /** Text inserted for a picked `{{placeholder}}`: the markup in templates, the value elsewhere */
    placeholderText: (name: string) => string;
    /** Value shown in the picker next to each placeholder; omitted inside templates */
    placeholderPreview?: (name: string) => string;
    onfiles: (files: File[]) => void;
    /** Pasted files the webview hides from JS; parent reads them from the OS clipboard */
    onclipboardfiles: () => void;
    /** Hotkeys owned by the parent (save, find, link); return true when handled */
    onhotkey?: (e: KeyboardEvent) => boolean;
  }

  let {
    content, baseDir, noteId, readonly = false, placeholder = '', notes, excludeId = null,
    onchange, onwikilink, placeholderText, placeholderPreview, onfiles, onclipboardfiles, onhotkey,
  }: Props = $props();

  let hostEl: HTMLElement | null = $state(null);
  let wrapEl: HTMLElement | null = $state(null);
  let editor: Editor | null = null;
  let lastEmitted = '';
  let isEmpty = $state(false);

  const isRelative = (url: string) => !/^([a-z][a-z0-9+.-]*:|\/\/|#|\/)/i.test(url);
  function resolveSrc(src: string): string {
    if (!baseDir || !isRelative(src)) return src;
    try {
      return convertFileSrc(`${baseDir}/${decodeURIComponent(src)}`);
    } catch {
      return src;
    }
  }

  onMount(() => {
    editor = new Editor({
      element: hostEl!,
      extensions: noteExtensions(resolveSrc),
      content,
      contentType: 'markdown',
      editable: !readonly,
      editorProps: {
        attributes: { spellcheck: 'false' },
        handleKeyDown: (_view, e) => onKeydown(e),
        handlePaste: (_view, e) => takePayload(e.clipboardData),
        handleClick: (_view, _pos, e) => onClick(e),
      },
      onUpdate: ({ editor: ed }) => {
        const md = ed.getMarkdown();
        isEmpty = ed.isEmpty;
        updateWikiState();
        updateImageBar();
        // Only real document changes reach the parent
        if (md === lastEmitted) return;
        lastEmitted = md;
        onchange(md);
      },
      onSelectionUpdate: () => { updateWikiState(); updateImageBar(); },
    });
    lastEmitted = content;
    isEmpty = editor.isEmpty;
    return () => editor?.destroy();
  });

  // Content changed outside the editor (history restore, merge, external file change, source mode)
  $effect(() => {
    const md = content;
    if (!editor || md === lastEmitted) return;
    lastEmitted = md;
    // Not undoable: Ctrl+Z must not bring back the replaced document
    editor.chain().setMeta('addToHistory', false).setContent(md, { contentType: 'markdown', emitUpdate: false }).run();
    isEmpty = editor.isEmpty;
  });

  // emitUpdate=false: setEditable must not fire onUpdate and push content to the store
  $effect(() => { editor?.setEditable(!readonly, false); });

  function takeFiles(files: File[]): boolean {
    if (readonly || files.length === 0) return false;
    onfiles(files);
    return true;
  }

  // Prefer real file bytes; otherwise let the parent read hidden file paths from the OS clipboard.
  function takePayload(dt: DataTransfer | null): boolean {
    if (readonly) return false;
    if (takeFiles(Array.from(dt?.files ?? []))) return true;
    if (!pasteHasHiddenFiles(dt)) return false;
    onclipboardfiles();
    return true;
  }

  function onKeydown(e: KeyboardEvent): boolean {
    if (wikiOpen) {
      if (e.key === 'Escape') { wikiOpen = false; return true; }
      if (wikiPicker?.handleKeydown(e)) return true;
    }
    if (tplOpen) {
      if (e.key === 'Escape') { tplOpen = false; return true; }
      if (tplPicker?.handleKeydown(e)) return true;
    }
    if (editor && takeHistory(e, (command) => {
      e.preventDefault();
      if (command === 'edit.undo') editor?.commands.undo();
      else editor?.commands.redo();
    })) return true;
    if (onhotkey?.(e)) return true;
    // Stock TipTap shortcuts follow event.key, so a custom binding would not replace them.
    if (stockHistoryChord(e) || stockMarkChord(e)) {
      e.preventDefault();
      return true;
    }
    return false;
  }

  /** Wiki atoms open on click; http links open with Ctrl/Cmd or when read-only, else edit the URL. */
  function onClick(e: MouseEvent): boolean {
    const target = e.target as HTMLElement;
    const wiki = target.closest('a.wiki');
    if (wiki) { onwikilink(wiki.getAttribute('data-target') ?? ''); return true; }
    const anchor = target.closest('a[href]');
    if (!anchor) return false;
    if (e.ctrlKey || e.metaKey || readonly) {
      const href = anchor.getAttribute('href') ?? '';
      if (/^https?:/i.test(href)) void api.system.openUrl(href);
    } else {
      openLinkForm();
    }
    return true;
  }

  export function focus() {
    editor?.commands.focus();
  }

  export function runAction(action: EditAction) {
    if (!editor || readonly) return;
    const c = editor.chain().focus();
    switch (action) {
      case 'h1': case 'h2': case 'h3': c.toggleHeading({ level: Number(action[1]) as 1 | 2 | 3 }).run(); break;
      case 'bold': c.toggleBold().run(); break;
      case 'italic': c.toggleItalic().run(); break;
      case 'strike': c.toggleStrike().run(); break;
      case 'code': c.toggleCode().run(); break;
      case 'ul': c.toggleBulletList().run(); break;
      case 'ol': c.toggleOrderedList().run(); break;
      case 'task': c.toggleTaskList().run(); break;
      case 'quote': c.toggleBlockquote().run(); break;
      case 'codeblock': c.toggleCodeBlock().run(); break;
      case 'hr': c.setHorizontalRule().run(); break;
      case 'link': openLinkForm(); break;
    }
  }

  /** Insert a Markdown snippet at the cursor. */
  export function insertMarkdown(md: string) {
    if (!editor || readonly) return;
    editor.chain().focus().insertContent(md, { contentType: 'markdown' }).run();
  }

  /** Move the caret to viewport coordinates (drop point). Returns false if outside the editor. */
  export function caretAtCoords(x: number, y: number): boolean {
    if (!editor) return false;
    const rect = hostEl?.querySelector('.ProseMirror')?.getBoundingClientRect();
    if (!rect || x < rect.left || x > rect.right || y < rect.top || y > rect.bottom) return false;
    const at = editor.view.posAtCoords({ left: x, top: y });
    if (!at) return false;
    editor.chain().focus().setTextSelection(at.pos).run();
    return true;
  }

  /** Select the first occurrence of `query` (within one text node) and scroll to it. */
  export function selectText(query: string) {
    if (!editor || !query) return;
    const q = query.toLowerCase();
    let found = -1;
    editor.state.doc.descendants((node, pos) => {
      if (found >= 0) return false;
      const i = node.isText ? (node.text ?? '').toLowerCase().indexOf(q) : -1;
      if (i >= 0) found = pos + i;
      return found < 0;
    });
    if (found < 0) return;
    editor.chain().focus().setTextSelection({ from: found, to: found + query.length }).scrollIntoView().run();
  }

  // ── Link form: shown under the caret for Ctrl+K / toolbar / click on a link ──
  let linkOpen = $state(false);
  let linkHref = $state('');
  let linkPos = $state({ x: 0, y: 0 });
  let linkInput: HTMLInputElement | null = $state(null);
  let hasLink = $state(false);

  function openLinkForm() {
    if (!editor || !wrapEl) return;
    const { from, to, empty } = editor.state.selection;
    const sel = editor.state.doc.textBetween(from, to, ' ');
    if (!empty && /^https?:\/\/\S+$/i.test(sel)) {
      editor.chain().focus().setLink({ href: sel }).run();
      return;
    }
    hasLink = editor.isActive('link');
    linkHref = editor.getAttributes('link').href ?? '';
    const caret = editor.view.coordsAtPos(from);
    const box = wrapEl.getBoundingClientRect();
    linkPos = { x: Math.max(0, caret.left - box.left), y: caret.bottom - box.top + 4 };
    linkOpen = true;
    tick().then(() => linkInput?.select());
  }

  function applyLink(e: SubmitEvent) {
    e.preventDefault();
    if (!editor) return;
    const href = linkHref.trim();
    const chain = editor.chain().focus().extendMarkRange('link');
    if (!href) chain.unsetLink().run();
    else if (editor.state.selection.empty && !editor.isActive('link')) {
      chain.insertContent({ type: 'text', text: href, marks: [{ type: 'link', attrs: { href } }] }).run();
    } else chain.setLink({ href }).run();
    linkOpen = false;
  }

  function removeLink() {
    editor?.chain().focus().extendMarkRange('link').unsetLink().run();
    linkOpen = false;
  }

  function closeLinkForm() {
    linkOpen = false;
    editor?.commands.focus();
  }

  // ── Wiki links: `[[` autocomplete in the current text block ─────────────────
  let wikiOpen = $state(false);
  let wikiQuery = $state('');
  let wikiIndex = $state(0);
  let wikiFrom = 0;
  let wikiPicker: WikiLinkPicker | null = $state(null);

  // ── Template placeholders: `{{` autocomplete in the current text block ─────
  let tplOpen = $state(false);
  let tplQuery = $state('');
  let tplIndex = $state(0);
  let tplFrom = 0;
  let tplPicker: PlaceholderPicker | null = $state(null);

  /** Text of the current block before the caret, or null when not in a collapsed text selection. */
  function textBeforeCaret(): { before: string; caretPos: number } | null {
    if (!editor || readonly) return null;
    const { $from: caret, empty } = editor.state.selection;
    if (!empty || !caret.parent.isTextblock) return null;
    return { before: caret.parent.textBetween(0, caret.parentOffset, undefined, '\ufffc'), caretPos: caret.pos };
  }

  /** Track an unclosed `[[` and `{{` before the caret inside the current text block. */
  function updateWikiState() {
    const ctx = textBeforeCaret();
    if (!ctx) { wikiOpen = false; tplOpen = false; return; }
    const { before, caretPos } = ctx;
    const open = unclosedMarkAt(before, WIKI_OPEN, WIKI_CLOSE);
    if (open >= 0) {
      wikiFrom = caretPos - (before.length - open);
      wikiQuery = before.slice(open + WIKI_OPEN.length);
      wikiIndex = 0;
      wikiOpen = true;
    } else {
      wikiOpen = false;
    }
    const tpl = unclosedMarkAt(before, TPL_OPEN, TPL_CLOSE);
    if (tpl >= 0 && !wikiOpen) {
      tplFrom = caretPos - (before.length - tpl);
      tplQuery = before.slice(tpl + TPL_OPEN.length);
      tplIndex = 0;
      tplOpen = true;
    } else {
      tplOpen = false;
    }
  }

  /** Replace the partial `{{query` with the placeholder markup or its value as plain text. */
  function pickPlaceholder(name: string) {
    if (!editor) return;
    const to = editor.state.selection.from;
    editor.chain().focus().insertContentAt({ from: tplFrom, to }, { type: 'text', text: placeholderText(name) }).run();
    tplOpen = false;
  }

  /** Plain text at the caret, e.g. `[[` to start a wiki link. */
  export function insertText(text: string) {
    editor?.chain().focus().insertContent(text).run();
  }

  /** Replace the partial `[[query` with a wiki link node. */
  function pickWikiLink(target: string, label?: string | null) {
    if (!editor) return;
    const to = editor.state.selection.from;
    editor.chain().focus()
      .insertContentAt({ from: wikiFrom, to }, [{ type: 'wikiLink', attrs: { target, label: label ?? null } }, { type: 'text', text: ' ' }])
      .run();
    wikiOpen = false;
  }

  let imgOpen = $state(false);
  let imgPos = $state({ x: 0, y: 0 });
  let imgName = $state('');

  function fileNameFromSrc(src: string): string {
    return decodeURIComponent(src.split(/[/\\]/).pop()?.split('?')[0] || '') || 'image';
  }

  function updateImageBar() {
    if (!editor || !wrapEl || !editor.isActive('image')) { imgOpen = false; return; }
    const src = String(editor.getAttributes('image').src ?? '');
    imgName = fileNameFromSrc(src);
    const { from } = editor.state.selection;
    const caret = editor.view.coordsAtPos(from);
    const box = wrapEl.getBoundingClientRect();
    imgPos = { x: Math.max(0, caret.left - box.left), y: caret.bottom - box.top + 8 };
    imgOpen = true;
  }

  async function downloadSelectedImage() {
    if (!editor) return;
    const src = String(editor.getAttributes('image').src ?? '');
    const name = fileNameFromSrc(src);
    if (noteId && isRelative(src)) {
      await downloadNoteAttachment(noteId, name);
      return;
    }
    const { save } = await import('@tauri-apps/plugin-dialog');
    const dest = await save({ defaultPath: name });
    if (!dest) return;
    const buf = new Uint8Array(await (await fetch(resolveSrc(src))).arrayBuffer());
    const { writeFile } = await import('@tauri-apps/plugin-fs');
    await writeFile(dest, buf);
  }
</script>

<div class="rich" bind:this={wrapEl}>
  <div class="host" bind:this={hostEl}></div>
  {#if isEmpty && !readonly && placeholder}
    <div class="placeholder">{placeholder}</div>
  {/if}
  {#if linkOpen}
    <form class="link-form" style="left: {linkPos.x}px; top: {linkPos.y}px" onsubmit={applyLink}>
      <Icon name="link" size={13} />
      <input
        bind:this={linkInput}
        bind:value={linkHref}
        type="text"
        placeholder="https://"
        onkeydown={(e) => { if (e.key === 'Escape') { e.preventDefault(); closeLinkForm(); } }}
      />
      <button type="submit" class="lf-btn" title={$t('note_tb_link')}><Icon name="check" size={13} /></button>
      {#if hasLink}
        <button type="button" class="lf-btn" title={$t('note_link_remove')} onclick={removeLink}><Icon name="x" size={13} /></button>
      {/if}
    </form>
  {/if}
  {#if wikiOpen}
    <WikiLinkPicker bind:this={wikiPicker} bind:index={wikiIndex} query={wikiQuery} {notes} {excludeId} onpick={pickWikiLink} />
  {:else if tplOpen}
    <PlaceholderPicker bind:this={tplPicker} bind:index={tplIndex} query={tplQuery} preview={placeholderPreview} onpick={pickPlaceholder} />
  {/if}
  {#if imgOpen}
    <div class="img-bar" style="left: {imgPos.x}px; top: {imgPos.y}px">
      <span class="img-name" title={imgName}>{imgName}</span>
      <button type="button" class="lf-btn" title={$t('note_att_download')} onclick={downloadSelectedImage}>
        <Icon name="download" size={13} />
      </button>
    </div>
  {/if}
</div>

<style>
  .rich {
    position: relative;
    flex: 1;
    display: flex;
    min-width: 0;
    min-height: 0;
  }
  .host {
    flex: 1;
    display: flex;
    min-width: 0;
    min-height: 0;
  }
  .placeholder {
    position: absolute;
    top: var(--sp-3);
    left: var(--sp-5);
    color: var(--text-3);
    font-size: 0.95rem;
    line-height: 1.7;
    pointer-events: none;
  }

  .link-form {
    position: absolute;
    z-index: 6;
    display: flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.25rem 0.4rem;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.25);
    color: var(--text-3);
  }
  /* Reset the global input chrome from base.css */
  .link-form input {
    width: 240px;
    height: 24px;
    padding: 0 0.3rem;
    background: none;
    border: none;
    outline: none;
    font-size: var(--fs-sm);
    color: var(--text);
  }
  .link-form input:focus { box-shadow: none; }
  .lf-btn {
    background: none;
    border: none;
    color: var(--text-2);
    cursor: pointer;
    border-radius: 4px;
    display: inline-flex;
    align-items: center;
    padding: 0.15rem 0.3rem;
  }
  .lf-btn:hover { background: var(--surface-2); color: var(--text); }

  .img-bar {
    position: absolute;
    z-index: 6;
    display: flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.2rem 0.4rem;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.25);
    max-width: 260px;
  }
  .img-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-xs);
    color: var(--text-2);
  }

  /* ── Document styles (shared look with the former read-only preview) ── */
  .host :global(.ProseMirror) {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    outline: none;
    padding: var(--sp-3) var(--sp-5);
    font-size: 0.95rem;
    line-height: 1.7;
    color: var(--text-body);
    word-wrap: break-word;
    white-space: pre-wrap;
  }
  .host :global(h1), .host :global(h2), .host :global(h3),
  .host :global(h4), .host :global(h5), .host :global(h6) {
    color: var(--text);
    margin: 1.2em 0 0.5em;
    line-height: 1.3;
    font-weight: var(--fw-bold);
  }
  .host :global(h1) { font-size: 1.6em; }
  .host :global(h2) { font-size: 1.35em; }
  .host :global(h3) { font-size: 1.15em; }
  .host :global(.ProseMirror > :first-child) { margin-top: 0; }
  .host :global(p) { margin: 0 0 0.8em; }
  .host :global(a) { color: var(--accent); text-decoration: none; cursor: pointer; }
  .host :global(a:hover) { text-decoration: underline; }
  .host :global(a.wiki) {
    padding: 0 0.15em;
    border-radius: 3px;
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  /* Entity mention `[[kind:id|Name]]`: chip look, opens the context card */
  .host :global(a.wiki-entity) {
    padding: 0 0.4em;
    border-radius: 999px;
    border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
    color: var(--text);
    background: var(--surface-2);
  }
  .host :global(code) {
    font-family: var(--font-mono);
    font-size: 0.88em;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0.05em 0.35em;
  }
  .host :global(pre) {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: var(--sp-3);
    overflow-x: auto;
    margin: 0 0 1em;
    font-family: var(--font-mono);
  }
  .host :global(pre code) { background: none; border: none; padding: 0; font-size: 0.85em; }
  .host :global(blockquote) {
    margin: 0 0 1em;
    padding: 0.2em 0 0.2em var(--sp-3);
    border-left: 3px solid var(--accent);
    color: var(--text-2);
  }
  .host :global(ul), .host :global(ol) { margin: 0 0 0.8em; padding-left: 1.5em; }
  .host :global(li) { margin: 0.15em 0; }
  .host :global(li > p) { margin: 0; }
  .host :global(ul[data-type="taskList"]) { list-style: none; padding-left: 0.2em; }
  .host :global(ul[data-type="taskList"] > li) { display: flex; align-items: flex-start; gap: 0.5em; }
  .host :global(ul[data-type="taskList"] > li > label) { flex-shrink: 0; margin-top: 0.3em; display: flex; }
  .host :global(ul[data-type="taskList"] > li > div) { flex: 1; min-width: 0; }
  .host :global(ul[data-type="taskList"] input[type="checkbox"]) {
    /* The mark of base.css, without the browser's margin */
    margin: 0;
  }
  .host :global(hr) { border: none; border-top: 1px solid var(--border); margin: 1.2em 0; }
  .host :global(img) { max-width: 100%; height: auto; border-radius: var(--radius-sm); vertical-align: middle; }
  .host :global([data-resize-container]) { max-width: 100%; vertical-align: middle; }
  .host :global([data-resize-wrapper]) { max-width: 100%; }
  .host :global([data-resize-wrapper] img) { display: block; }
  .host :global([data-resize-handle]) {
    width: 10px;
    height: 10px;
    background: var(--accent);
    border: 1px solid var(--surface);
    border-radius: 2px;
    z-index: 2;
    display: none;
  }
  .host :global([data-resize-handle="bottom-right"]) { cursor: nwse-resize; transform: translate(30%, 30%); }
  .host :global([data-resize-handle="bottom-left"]) { cursor: nesw-resize; transform: translate(-30%, 30%); }
  .host :global([data-resize-handle="top-right"]) { cursor: nesw-resize; transform: translate(30%, -30%); }
  .host :global([data-resize-handle="top-left"]) { cursor: nwse-resize; transform: translate(-30%, -30%); }
  .host :global(.ProseMirror-selectednode [data-resize-handle]),
  .host :global([data-resize-container].ProseMirror-selectednode [data-resize-handle]) {
    display: block;
  }
  .host :global(img.ProseMirror-selectednode) { outline: 2px solid var(--accent); }
  .host :global([data-resize-container].ProseMirror-selectednode) { outline: 2px solid var(--accent); border-radius: var(--radius-sm); }
  .host :global(.ProseMirror-selectednode) { outline: 2px solid var(--accent); border-radius: 3px; }
  .host :global(.tableWrapper) { overflow-x: auto; margin: 0 0 1em; }
  .host :global(table) { border-collapse: collapse; font-size: 0.92em; table-layout: fixed; width: 100%; }
  .host :global(th), .host :global(td) {
    border: 1px solid var(--border);
    padding: 0.3em 0.6em;
    text-align: left;
    vertical-align: top;
    position: relative;
  }
  .host :global(th > p), .host :global(td > p) { margin: 0; }
  .host :global(th) { background: var(--surface-2); color: var(--text); }
  .host :global(.selectedCell::after) {
    content: '';
    position: absolute;
    inset: 0;
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    pointer-events: none;
  }
</style>
