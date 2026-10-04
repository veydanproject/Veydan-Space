<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- WYSIWYG editor over the desktop tiptap extensions; Markdown stays the storage format. -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { Editor } from '@tiptap/core';
  import { noteExtensions, unclosedMarkAt, TPL_CLOSE, TPL_OPEN, WIKI_CLOSE, WIKI_OPEN, type ResolveSrc } from '$lib/notes/tiptap-ext';
  import { stockHistoryChord, takeHistory } from '$lib/core/keybindings';
  import type { EditAction } from '$lib/notes/markdown-edit';
  import { portal } from '$lib/core/portal';
  import { t } from '$lib/core/mobile/i18n';

  interface Props {
    content: string;
    resolveSrc: ResolveSrc;
    placeholder: string;
    onchange: (md: string) => void;
    /** Text typed after an unclosed `[[`, or null when no wiki link is being typed. */
    onwiki: (query: string | null) => void;
    onwikilink: (target: string) => void;
    /** Text typed after an unclosed `{{`, or null when no placeholder is being typed. */
    ontpl?: (query: string | null) => void;
    /** A new note: the caret goes into the body and the keyboard opens. */
    autofocus?: boolean;
  }

  let { content, resolveSrc, placeholder, onchange, onwiki, onwikilink, ontpl, autofocus = false }: Props = $props();

  let hostEl: HTMLElement | null = $state(null);
  let editor: Editor | null = null;
  let lastEmitted = '';
  let isEmpty = $state(false);
  let wikiFrom = 0;
  let tplFrom = 0;
  let lightboxSrc = $state('');

  const MIN_IMG_W = 40;
  const TAP_MS = 300;

  type Pinch = {
    startDist: number;
    startW: number;
    ratio: number;
    img: HTMLImageElement;
  };

  let pinch: Pinch | null = null;
  let didPinch = false;
  let lastTapAt = 0;
  let lastTapImg: HTMLImageElement | null = null;

  onMount(() => {
    editor = new Editor({
      element: hostEl!,
      extensions: noteExtensions(resolveSrc),
      content,
      contentType: 'markdown',
      autofocus: autofocus ? 'end' : false,
      editorProps: {
        attributes: { spellcheck: 'false' },
        handleKeyDown: (_view, e) => {
          if (!editor) return false;
          const taken = takeHistory(e, (command) => {
            e.preventDefault();
            if (command === 'edit.undo') editor?.commands.undo();
            else editor?.commands.redo();
          });
          if (taken) return true;
          if (stockHistoryChord(e)) {
            e.preventDefault();
            return true;
          }
          return false;
        },
        handleClick: (_view, _pos, e) => onClick(e),
        handleDoubleClick: (_view, _pos, e) => onDblClick(e),
      },
      onUpdate: ({ editor: ed }) => {
        const md = ed.getMarkdown();
        isEmpty = ed.isEmpty;
        updateWikiState();
        if (md === lastEmitted) return;
        lastEmitted = md;
        onchange(md);
      },
      onSelectionUpdate: updateWikiState,
    });
    lastEmitted = content;
    isEmpty = editor.isEmpty;
    const host = hostEl!;
    host.addEventListener('touchstart', onTouchStart, { passive: false });
    host.addEventListener('touchmove', onTouchMove, { passive: false });
    host.addEventListener('touchend', onTouchEnd);
    host.addEventListener('touchcancel', onTouchEnd);
    document.addEventListener('touchend', finishHandleResize);
    document.addEventListener('touchcancel', finishHandleResize);
    return () => {
      editor?.destroy();
      host.removeEventListener('touchstart', onTouchStart);
      host.removeEventListener('touchmove', onTouchMove);
      host.removeEventListener('touchend', onTouchEnd);
      host.removeEventListener('touchcancel', onTouchEnd);
      document.removeEventListener('touchend', finishHandleResize);
      document.removeEventListener('touchcancel', finishHandleResize);
    };
  });

  // Content replaced outside the editor (history restore, remote edit).
  $effect(() => {
    const md = content;
    if (!editor || md === lastEmitted) return;
    lastEmitted = md;
    editor.chain().setMeta('addToHistory', false).setContent(md, { contentType: 'markdown', emitUpdate: false }).run();
    isEmpty = editor.isEmpty;
  });

  $effect(() => {
    if (!lightboxSrc) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') lightboxSrc = ''; };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  function imageEl(target: EventTarget | null): HTMLImageElement | null {
    if (!(target instanceof Element)) return null;
    if (target.closest('[data-resize-handle]')) return null;
    const box = target.closest('[data-node="image"]');
    if (box instanceof HTMLElement) return box.querySelector('img');
    return target instanceof HTMLImageElement ? target : target.closest('img');
  }

  function imagePos(el: HTMLElement): number | null {
    if (!editor) return null;
    const box = (el.closest('[data-resize-container]') as HTMLElement | null) ?? el;
    try {
      const pos = editor.view.posAtDOM(box, 0);
      if (editor.state.doc.nodeAt(pos)?.type.name === 'image') return pos;
    } catch { /* use coords */ }
    const r = box.getBoundingClientRect();
    const at = editor.view.posAtCoords({ left: r.left + 4, top: r.top + 4 });
    if (!at) return null;
    return editor.state.doc.nodeAt(at.inside)?.type.name === 'image' ? at.inside : null;
  }

  function maxImgWidth(): number {
    return hostEl?.querySelector('.ProseMirror')?.clientWidth || 400;
  }

  function applyImgWidth(img: HTMLImageElement, width: number, ratio: number) {
    const w = Math.round(Math.min(maxImgWidth(), Math.max(MIN_IMG_W, width)));
    img.style.width = `${w}px`;
    img.style.height = `${Math.round(w * ratio)}px`;
    return w;
  }

  function commitImgSize(img: HTMLImageElement) {
    if (!editor) return;
    const pos = imagePos(img);
    if (pos == null) return;
    const width = img.offsetWidth;
    const height = img.offsetHeight;
    editor.chain().setNodeSelection(pos).updateAttributes('image', { width, height }).run();
  }

  function openLightbox(img: HTMLImageElement) {
    lightboxSrc = img.currentSrc || img.src;
  }

  function touchDist(a: Touch, b: Touch) {
    return Math.hypot(a.clientX - b.clientX, a.clientY - b.clientY);
  }

  function onTouchStart(e: TouchEvent) {
    if (e.touches.length !== 2) return;
    const img = imageEl(e.touches[0].target) ?? imageEl(e.touches[1].target);
    if (!img) return;
    const w = img.offsetWidth;
    const h = img.offsetHeight || w;
    pinch = { startDist: touchDist(e.touches[0], e.touches[1]), startW: w, ratio: h / w, img };
    didPinch = true;
    lastTapAt = 0;
    lastTapImg = null;
    const pos = imagePos(img);
    if (pos != null) editor?.chain().setNodeSelection(pos).run();
    e.preventDefault();
  }

  function onTouchMove(e: TouchEvent) {
    if (!pinch || e.touches.length < 2 || pinch.startDist <= 0) return;
    e.preventDefault();
    applyImgWidth(pinch.img, pinch.startW * (touchDist(e.touches[0], e.touches[1]) / pinch.startDist), pinch.ratio);
  }

  function onTouchEnd(e: TouchEvent) {
    if (pinch && e.touches.length < 2) {
      const img = pinch.img;
      const changed = Math.abs(img.offsetWidth - pinch.startW) >= 4;
      pinch = null;
      if (changed) commitImgSize(img);
    }
    if (didPinch) {
      if (e.touches.length === 0) didPinch = false;
      return;
    }
    if (e.touches.length !== 0 || e.changedTouches.length !== 1) return;
    const img = imageEl(e.target);
    if (!img) { lastTapAt = 0; lastTapImg = null; return; }
    const now = Date.now();
    if (lastTapImg === img && now - lastTapAt < TAP_MS) {
      e.preventDefault();
      openLightbox(img);
      lastTapAt = 0;
      lastTapImg = null;
      return;
    }
    lastTapAt = now;
    lastTapImg = img;
  }

  // TipTap handle resize listens for mouseup but not touchend
  function finishHandleResize() {
    if (hostEl?.querySelector('[data-resize-state="true"]')) {
      document.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }));
    }
  }

  function onClick(e: MouseEvent): boolean {
    const wiki = (e.target as HTMLElement).closest('a.wiki');
    if (!wiki) return false;
    onwikilink(wiki.getAttribute('data-target') ?? '');
    return true;
  }

  function onDblClick(e: MouseEvent): boolean {
    const img = imageEl(e.target);
    if (!img) return false;
    openLightbox(img);
    return true;
  }

  /** Track an unclosed `[[` and `{{` before the caret inside the current text block. */
  function updateWikiState() {
    if (!editor) return;
    const { $from: caret, empty } = editor.state.selection;
    if (!empty || !caret.parent.isTextblock) { onwiki(null); ontpl?.(null); return; }
    const before = caret.parent.textBetween(0, caret.parentOffset, undefined, '\ufffc');
    const open = unclosedMarkAt(before, WIKI_OPEN, WIKI_CLOSE);
    if (open >= 0) {
      wikiFrom = caret.pos - (before.length - open);
      onwiki(before.slice(open + WIKI_OPEN.length));
      ontpl?.(null);
      return;
    }
    onwiki(null);
    const tpl = unclosedMarkAt(before, TPL_OPEN, TPL_CLOSE);
    if (tpl < 0) { ontpl?.(null); return; }
    tplFrom = caret.pos - (before.length - tpl);
    ontpl?.(before.slice(tpl + TPL_OPEN.length));
  }

  /** Replace the partial `{{query` with plain text (markup or expanded value). */
  export function pickPlaceholder(text: string) {
    if (!editor) return;
    const to = editor.state.selection.from;
    editor.chain().focus().insertContentAt({ from: tplFrom, to }, { type: 'text', text }).run();
    ontpl?.(null);
  }

  /** Replace the partial `[[query` with a wiki link node. */
  export function pickWikiLink(target: string, label?: string | null) {
    if (!editor) return;
    const to = editor.state.selection.from;
    editor.chain().focus()
      .insertContentAt({ from: wikiFrom, to }, [{ type: 'wikiLink', attrs: { target, label: label ?? null } }, { type: 'text', text: ' ' }])
      .run();
    onwiki(null);
  }

  export function insertMarkdown(md: string) {
    editor?.chain().focus().insertContent(md, { contentType: 'markdown' }).run();
  }

  /** Toolbar formatting; `link` is handled by the page via linkHref/setLink. */
  export function runAction(action: EditAction) {
    if (!editor) return;
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
      case 'link': break;
    }
  }

  /** Href of the link at the caret, or the selected text when it is a URL. */
  export function linkHref(): string {
    if (!editor) return '';
    const { from, to, empty } = editor.state.selection;
    const sel = empty ? '' : editor.state.doc.textBetween(from, to, ' ');
    return editor.getAttributes('link').href ?? (/^https?:\/\/\S+$/i.test(sel) ? sel : '');
  }

  /** Apply or clear the link on the selection; inserts the URL as text when nothing is selected. */
  export function setLink(href: string) {
    if (!editor) return;
    const chain = editor.chain().focus().extendMarkRange('link');
    if (!href) chain.unsetLink().run();
    else if (editor.state.selection.empty && !editor.isActive('link')) {
      chain.insertContent({ type: 'text', text: href, marks: [{ type: 'link', attrs: { href } }] }).run();
    } else chain.setLink({ href }).run();
  }

  /** Plain text at the caret, e.g. `[[` to start a wiki link. */
  export function insertText(text: string) {
    editor?.chain().focus().insertContent(text).run();
  }

  /** Viewport box of the caret line, so the page can keep it above the keyboard. */
  export function caretBox(): { top: number; bottom: number } | null {
    if (!editor) return null;
    try {
      const c = editor.view.coordsAtPos(editor.state.selection.head);
      return { top: c.top, bottom: c.bottom };
    } catch {
      return null;
    }
  }
</script>

<div class="rich">
  <div class="host" bind:this={hostEl}></div>
  {#if isEmpty && placeholder}
    <div class="placeholder">{placeholder}</div>
  {/if}
</div>
{#if lightboxSrc}
  <button type="button" class="lightbox" use:portal onclick={() => (lightboxSrc = '')} aria-label={$t('common_close')}>
    <img src={lightboxSrc} alt="" />
  </button>
{/if}

<style>
  .rich {
    position: relative;
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .host { flex: 1; display: flex; min-width: 0; min-height: 0; }
  .placeholder {
    position: absolute;
    top: var(--sp-2);
    left: 0;
    color: var(--text-3);
    pointer-events: none;
  }

  .host :global(.ProseMirror) {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    outline: none;
    padding: var(--sp-2) 0;
    font-size: 15px;
    line-height: 1.6;
    word-wrap: break-word;
    white-space: pre-wrap;
    user-select: text;
    -webkit-user-select: text;
  }
  .host :global(h1), .host :global(h2), .host :global(h3) { margin: 1em 0 0.4em; line-height: 1.3; }
  .host :global(h1) { font-size: 1.5em; }
  .host :global(h2) { font-size: 1.3em; }
  .host :global(h3) { font-size: 1.15em; }
  .host :global(.ProseMirror > :first-child) { margin-top: 0; }
  .host :global(p) { margin: 0 0 0.7em; }
  .host :global(a) { color: var(--accent-text); text-decoration: none; }
  .host :global(a.wiki) {
    padding: 0 0.15em;
    border-radius: 3px;
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
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
    overflow-x: auto;
    margin: 0 0 1em;
    padding: var(--sp-3);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-md, 8px);
    font-family: var(--font-mono);
  }
  .host :global(pre code) { background: none; border: none; padding: 0; font-size: 13px; }
  .host :global(blockquote) {
    margin: 0 0 1em;
    padding-left: var(--sp-3);
    border-left: 3px solid var(--border-2);
    color: var(--text-2);
  }
  .host :global(ul), .host :global(ol) { margin: 0 0 0.7em; padding-left: 1.4em; }
  .host :global(li > p) { margin: 0; }
  .host :global(ul[data-type="taskList"]) { list-style: none; padding-left: 0; }
  .host :global(ul[data-type="taskList"] > li) { display: flex; align-items: flex-start; gap: 0.5em; }
  .host :global(ul[data-type="taskList"] > li > label) { flex-shrink: 0; margin-top: 0.2em; display: flex; }
  .host :global(ul[data-type="taskList"] > li > div) { flex: 1; min-width: 0; }
  .host :global(ul[data-type="taskList"] input[type="checkbox"]) {
    width: 20px;
    height: 20px;
    margin: 0;
  }
  .host :global(hr) { border: none; border-top: 1px solid var(--border); margin: 1em 0; }
  .host :global(img) {
    max-width: 100%;
    height: auto;
    border-radius: var(--radius-md, 8px);
    vertical-align: middle;
    user-select: none;
    -webkit-user-select: none;
  }
  .host :global(img:not([style*="width"])) { max-width: min(100%, 240px); }
  .host :global([data-resize-container]), .host :global([data-resize-wrapper]) { max-width: 100%; }
  .host :global([data-resize-container]) { touch-action: pan-y; }
  .host :global([data-resize-wrapper] img) { display: block; }
  .host :global([data-resize-handle]) {
    width: 22px;
    height: 22px;
    background: var(--accent);
    border: 2px solid var(--surface);
    border-radius: 4px;
    z-index: 2;
    display: none;
    touch-action: none;
  }
  .host :global([data-resize-handle="bottom-right"]) { transform: translate(35%, 35%); }
  .host :global([data-resize-handle="bottom-left"]) { transform: translate(-35%, 35%); }
  .host :global([data-resize-handle="top-right"]) { transform: translate(35%, -35%); }
  .host :global([data-resize-handle="top-left"]) { transform: translate(-35%, -35%); }
  .host :global(.ProseMirror-selectednode [data-resize-handle]),
  .host :global([data-resize-container].ProseMirror-selectednode [data-resize-handle]) {
    display: block;
  }
  .host :global(.ProseMirror-selectednode) { outline: 2px solid var(--accent); border-radius: 3px; }
  .host :global([data-resize-container].ProseMirror-selectednode) {
    outline: 2px solid var(--accent);
    border-radius: var(--radius-md, 8px);
  }
  .lightbox {
    position: fixed;
    inset: 0;
    z-index: 80;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
    height: 100%;
    margin: 0;
    padding: var(--sat) var(--sar) var(--sab) var(--sal);
    border: 0;
    border-radius: 0;
    background: rgba(0, 0, 0, 0.92);
    touch-action: none;
    transition: none;
  }
  .lightbox img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
    border-radius: 0;
  }
  .host :global(.tableWrapper) { overflow-x: auto; margin: 0 0 1em; }
  .host :global(table) { border-collapse: collapse; font-size: 0.92em; }
  .host :global(th), .host :global(td) { border: 1px solid var(--border); padding: 4px 8px; vertical-align: top; }
  .host :global(th > p), .host :global(td > p) { margin: 0; }
  .host :global(th) { background: var(--surface-2); }
</style>
