<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { ask } from '$lib/core/ui/confirm.svelte';
  import { onDestroy, onMount, tick, untrack } from 'svelte';
  import { goto, replaceState } from '$app/navigation';
  import { page } from '$app/state';
  import Icon from '$lib/core/Icon.svelte';
  import {
    api, formatError, onNoteRemote,
    type NavChild, type Note, type NoteAttachment, type NoteChip, type NoteListItem, type NoteTag,
  } from '$lib/notes/mobile/api';
  import { locale, t } from '$lib/core/mobile/i18n';
  import { hasErrorCode } from '$lib/core/utils';
  import { api as shared, DEFAULT_ATTACHMENT_POLICY, type AttachmentTransfer } from '$lib/notes/api';
  import { onAttachmentTransfer } from '$lib/notes/attachmentTransfer';
  import { AttachmentUrls, attachmentHref } from '$lib/notes/mobile/markdown';
  import { markdownDestination, repairAttachmentLinks } from '$lib/notes/markdown';
  import { fmtDateTime, fmtSize, loadEditorMode, saveEditorMode, type EditorMode } from '$lib/notes/mobile/notes-editor';
  import { onKeyboard } from '$lib/core/mobile/keyboard';
  import { leave } from '$lib/core/ui/back';
  import NoteRichEditor from '$lib/notes/mobile/NoteRichEditor.svelte';
  import NoteFormatBar from '$lib/notes/mobile/NoteFormatBar.svelte';
  import { applyAction, type EditAction } from '$lib/notes/markdown-edit';
  import WikiLinkSheet from '$lib/notes/mobile/WikiLinkSheet.svelte';
  import NoteActionsSheet from '$lib/notes/mobile/NoteActionsSheet.svelte';
  import NoteHistorySheet from '$lib/notes/mobile/NoteHistorySheet.svelte';
  import NoteLinksSheet from '$lib/notes/mobile/NoteLinksSheet.svelte';
  import MoveFolderSheet from '$lib/notes/mobile/MoveFolderSheet.svelte';
  import WorkspaceSheet from '$lib/notes/mobile/WorkspaceSheet.svelte';
  import NoteLabelSheet from '$lib/notes/mobile/NoteLabelSheet.svelte';
  import NoteAttachSheet from '$lib/notes/mobile/NoteAttachSheet.svelte';
  import NoteConflictSheet from '$lib/notes/mobile/NoteConflictSheet.svelte';
  import MediaCapture from '$lib/notes/media/MediaCapture.svelte';
  import { mediaKindOf } from '$lib/notes/media/kind';
  import type { CaptureFile, MediaItem, MediaKind } from '$lib/notes/media/types';
  import ChipMark from '$lib/core/ui/ChipMark.svelte';
  import { extractWikiTargets, unclosedMarkAt, wikiMarkup, TPL_CLOSE, TPL_OPEN, WIKI_CLOSE, WIKI_OPEN } from '$lib/notes/tiptap-ext';
  import { isEntityBinding, parseBinding } from '$lib/core/bindings';
  import { canBindEntities, contextKeys } from '$lib/notes/context';
  import { directory } from '$lib/core/directory';
  import { TEMPLATES_FOLDER } from '$lib/notes/filter';
  import { findTag, tagChoice } from '$lib/notes/tag-name';
  import NoteContextSheet from '$lib/notes/mobile/NoteContextSheet.svelte';
  import PlaceholderSheet from '$lib/notes/mobile/PlaceholderSheet.svelte';

  // "new" means nothing is stored yet; the note is created on the first edit.
  let id = $state(page.params.id ?? 'new');
  let title = $state('');
  let content = $state('');
  let tags = $state<string[]>([]);
  let chips = $state<NoteChip[]>([]);
  /** `kind:value` bindings of the saved note */
  let bindings = $state<string[]>([]);
  let pinned = $state(false);
  let archived = $state(false);
  let createdAt = $state('');
  let updatedAt = $state('');
  let error = $state('');
  let status = $state<'idle' | 'dirty' | 'saving' | 'saved' | 'failed'>('idle');
  let editingTitle = $state(false);
  // Attachments are loaded before the editor mounts so image sources resolve.
  let ready = $state(untrack(() => id) === 'new');

  let mode = $state<EditorMode>(loadEditorMode());
  let sheet = $state<'none' | 'actions' | 'links' | 'history' | 'move' | 'attach' | 'conflict' | 'workspace' | 'labels' | 'context'>('none');
  // Unresolved sync conflict on this note; drives the banner.
  let hasConflict = $state(false);
  let allTags = $state<NoteTag[]>([]);
  let allNotes = $state<NoteListItem[]>([]);
  let folders = $state<NavChild[]>([]);
  let workspaces = $state<NavChild[]>([]);
  let profiles = $state<NavChild[]>([]);
  /** Bindings added on a note that is not saved yet. */
  let extraBindings = $state<string[]>([]);
  /** Inherited bindings the user turned off before the first save. */
  let droppedBindings = $state<string[]>([]);
  /** Folders picked before the note is saved. */
  let extraFolders = $state<string[]>([]);
  /** Inherited folder the user removed before the first save. */
  let droppedFolders = $state<string[]>([]);
  let attachments = $state<NoteAttachment[]>([]);
  /** Audio and video attachments as seen by the capture module. */
  const mediaItems = $derived<MediaItem[]>(
    attachments.flatMap((a) => {
      const kind = mediaKindOf(a.name);
      return kind ? [{ name: a.name, size: a.size, kind, present: a.present }] : [];
    }),
  );
  let thumbs = $state<Record<string, string>>({});
  let urls = $state(new AttachmentUrls(untrack(() => id)));
  let wikiQuery = $state('');
  let wikiOpen = $state(false);
  let wikiStart = 0;
  let busy = $state(false);
  let linkOpen = $state(false);
  let linkHref = $state('');
  let linkInput = $state<HTMLInputElement>();

  let rich = $state<NoteRichEditor>();
  let body = $state<HTMLTextAreaElement>();
  let scrollEl = $state<HTMLDivElement>();
  /** IME overlap; 0 while the keyboard is closed. */
  let kb = $state(0);
  /** Large vault transfers of this note's attachments, keyed by `note_id/name`. */
  let transfers = $state<Map<string, AttachmentTransfer>>(new Map());
  /** Vault-only attachments at or above this size ask before downloading. */
  const askAboveBytes = shared.notes.attachmentPolicyGet()
    .then((p) => p.ask_above_mib * 1024 * 1024)
    .catch(() => DEFAULT_ATTACHMENT_POLICY.ask_above_mib * 1024 * 1024);
  /** Note already prompted for downloads in this session. */
  let fetchAsked: string | null = null;
  /** Vault-only attachments offered for download in the card under the header. */
  let fetchOffer = $state<NoteAttachment[]>([]);
  const fetchOfferBytes = $derived(fetchOffer.reduce((n, a) => n + a.size, 0));

  let saveTimer: ReturnType<typeof setTimeout>;
  let saving: Promise<void> | null = null;
  /** Hash and body last loaded or saved. A watcher event that still matches them is our echo. */
  let baseHash = $state<string | null>(null);
  let knownBody = $state('');
  let externalChange = $state(false);
  let saveInFlight = false;
  let remoteDuringSave = false;
  const CARET_GAP = 16;

  /** Keep the caret line inside the area above the keyboard. */
  function revealCaret() {
    const sc = scrollEl;
    const box = editorCaretBox();
    if (!sc || !box) return;
    const frame = visibleFrame(sc);
    if (frame.bottom <= frame.top) return;
    if (box.bottom > frame.bottom - CARET_GAP) sc.scrollTop += box.bottom - (frame.bottom - CARET_GAP);
    else if (box.top < frame.top + CARET_GAP) sc.scrollTop -= frame.top + CARET_GAP - box.top;
  }

  /** Scrollport clipped to the visual viewport, so the keyboard does not count as visible. */
  function visibleFrame(sc: HTMLElement): { top: number; bottom: number } {
    const view = sc.getBoundingClientRect();
    const vv = window.visualViewport;
    const top = Math.max(view.top, vv?.offsetTop ?? 0);
    const bottom = Math.min(view.bottom, (vv?.offsetTop ?? 0) + (vv?.height ?? view.bottom));
    return { top, bottom };
  }

  function editorCaretBox(): { top: number; bottom: number } | null {
    if (mode === 'rich') {
      const pm = scrollEl?.querySelector('.ProseMirror');
      const node = document.getSelection()?.anchorNode;
      if (!pm || !node || !pm.contains(node)) return null;
      return rich?.caretBox() ?? null;
    }
    if (document.activeElement !== body || !body) return null;
    return textareaCaretBox(body);
  }

  /** Viewport box of the textarea caret. The selection API has no rect for it. */
  function textareaCaretBox(ta: HTMLTextAreaElement): { top: number; bottom: number } {
    const pos = ta.selectionDirection === 'backward' ? ta.selectionStart : ta.selectionEnd;
    const cs = getComputedStyle(ta);
    const mirror = document.createElement('div');
    mirror.style.cssText = 'position:fixed;left:0;top:0;visibility:hidden;pointer-events:none;white-space:pre-wrap;overflow-wrap:break-word;overflow:hidden';
    mirror.style.boxSizing = 'border-box';
    mirror.style.width = `${ta.clientWidth}px`;
    mirror.style.font = cs.font;
    mirror.style.lineHeight = cs.lineHeight;
    mirror.style.letterSpacing = cs.letterSpacing;
    mirror.style.padding = cs.padding;
    mirror.style.border = cs.border;
    mirror.textContent = ta.value.slice(0, pos);
    const mark = document.createElement('span');
    mark.textContent = '\u200b';
    mirror.append(mark);
    document.body.append(mirror);
    const top = ta.getBoundingClientRect().top + mark.offsetTop - ta.scrollTop;
    const height = mark.offsetHeight || parseFloat(cs.lineHeight) || 24;
    mirror.remove();
    return { top, bottom: top + height };
  }

  $effect(() => {
    const sc = scrollEl;
    const stop = onKeyboard((n) => {
      kb = n;
      requestAnimationFrame(revealCaret);
    });
    const onSel = () => requestAnimationFrame(revealCaret);
    document.addEventListener('selectionchange', onSel);
    sc?.addEventListener('input', onSel);
    return () => {
      stop();
      document.removeEventListener('selectionchange', onSel);
      sc?.removeEventListener('input', onSel);
    };
  });

  const folderChip = $derived(chips.find((c) => c.kind === 'folder'));
  const isNew = $derived(id === 'new');
  const PROFILE_COLOR = '#8b7bff';

  /** Folder or workspace/profile the list was showing when New was tapped. */
  const createContext = $derived.by(() => {
    if (!isNew) return { kind: '', id: '' };
    return {
      kind: page.url.searchParams.get('kind') ?? '',
      id: page.url.searchParams.get('id') ?? '',
    };
  });
  const inheritedBindings = $derived(
    (createContext.kind === 'workspace' || createContext.kind === 'profile') && createContext.id
      ? [`${createContext.kind}:${createContext.id}`]
      : [],
  );
  const draftBindings = $derived(
    [...new Set([...inheritedBindings, ...extraBindings])].filter((b) => !droppedBindings.includes(b)),
  );
  const inheritedFolderId = $derived(createContext.kind === 'folder' && createContext.id ? createContext.id : '');
  const folderIds = $derived.by(() => {
    if (!isNew) return chips.filter((c) => c.kind === 'folder').map((c) => c.id);
    const ids = new Set(extraFolders);
    if (inheritedFolderId && !droppedFolders.includes(inheritedFolderId)) ids.add(inheritedFolderId);
    for (const id of droppedFolders) ids.delete(id);
    return [...ids];
  });
  const activeBindings = $derived(
    isNew
      ? draftBindings
      : chips
          .filter((c) => c.kind === 'workspace' || c.kind === 'profile' || c.kind === 'domain')
          .map((c) => `${c.kind}:${c.id}`),
  );
  const workspaceIds = $derived(
    isNew
      ? draftBindings.filter((b) => b.startsWith('workspace:')).map((b) => b.slice('workspace:'.length))
      : chips.filter((c) => c.kind === 'workspace').map((c) => c.id),
  );

  function bindingChip(binding: string): NoteChip | null {
    if (binding.startsWith('workspace:')) {
      const wsId = binding.slice('workspace:'.length);
      const w = workspaces.find((x) => x.id === wsId);
      return w ? { kind: 'workspace', id: wsId, label: w.name, color: w.color } : null;
    }
    if (binding.startsWith('profile:')) {
      const profileId = binding.slice('profile:'.length);
      const p = profiles.find((x) => x.id === profileId);
      return p ? { kind: 'profile', id: profileId, label: p.name, color: PROFILE_COLOR } : null;
    }
    return null;
  }

  function folderChipOf(folderId: string): NoteChip | null {
    const f = folders.find((x) => x.id === folderId);
    return f ? { kind: 'folder', id: folderId, label: f.name, color: f.color } : null;
  }

  const shownChips = $derived(
    isNew
      ? [
          ...folderIds.map(folderChipOf).filter((c): c is NoteChip => !!c),
          ...draftBindings.map(bindingChip).filter((c): c is NoteChip => !!c),
        ]
      : chips.filter((c) => c.kind !== 'tag'),
  );

  /** Re-read chips and bindings after a metadata change. */
  async function refreshMeta() {
    const n = await api.notes.get(id);
    chips = n.chips;
    bindings = n.bindings;
  }

  // ── Context: entity bindings and `[[kind:id]]` mentions ──
  /** Binding to highlight in the context sheet, e.g. after tapping a mention */
  let contextFocus = $state<string | null>(null);
  const mentions = $derived(extractWikiTargets(content).filter(isEntityBinding));
  const entityBindings = $derived((isNew ? draftBindings : bindings).filter(isEntityBinding));
  const contextCount = $derived(contextKeys(entityBindings, mentions, isNew ? null : id).length);
  /** The Context sheet is offered where it can have content: bound objects, or kinds to bind. */
  const showContext = $derived(contextCount > 0 || canBindEntities());

  $effect(() => {
    void directory.ensureLoaded();
  });

  function openContext(focus: string | null = null) {
    contextFocus = focus;
    sheet = 'context';
  }

  function applyNote(n: Note) {
    title = n.title;
    content = repairAttachmentLinks(n.content);
    knownBody = n.content;
    baseHash = n.contentHash;
    tags = n.tags;
    chips = n.chips;
    bindings = n.bindings;
    pinned = n.pinned;
    archived = n.archived;
    createdAt = n.created_at;
    updatedAt = n.updated_at;
  }

  async function load() {
    if (id === 'new') return;
    try {
      const n = await api.notes.get(id);
      await loadAttachments();
      applyNote(n);
      if (status !== 'dirty' && status !== 'saving') status = 'saved';
      ready = true;
      void promptFetch();
    } catch (e) {
      error = formatError(e);
    }
  }

  /** Once per note: show the download card for vault-only attachments above the prompt size. */
  async function promptFetch() {
    if (fetchAsked === id) return;
    fetchAsked = id;
    const minSize = await askAboveBytes;
    fetchOffer = attachments.filter((a) => !a.present && a.size >= minSize);
  }

  async function fetchOffered() {
    const noteId = id;
    const list = fetchOffer;
    fetchOffer = [];
    for (const a of list) {
      if (id !== noteId) return;
      await fetchAttachment(a);
    }
  }

  async function fetchAttachment(a: NoteAttachment) {
    try {
      await api.attachments.fetch(id, a.name);
      await loadAttachments();
    } catch (err) {
      error = formatError(err);
    }
  }

  async function loadAttachments() {
    if (id === 'new') return;
    attachments = await api.attachments.list(id);
    const names = new Set(attachments.map((a) => a.name));
    for (const name of Object.keys(thumbs)) {
      if (!names.has(name)) {
        urls.forget(name);
        delete thumbs[name];
      }
    }
    for (const a of attachments) {
      if (a.is_image && a.present && !thumbs[a.name]) {
        const url = await urls.get(a.name);
        if (url) thumbs[a.name] = url;
      }
    }
  }

  function loadCatalogs() {
    api.notes.tags().then((x) => (allTags = x)).catch(() => {});
    api.notes.list().then((x) => (allNotes = x)).catch(() => {});
    api.notes.nav().then((x) => {
      folders = x.folders;
      workspaces = x.all_workspaces;
      profiles = x.all_profiles;
    }).catch(() => {});
  }

  /** Own write echoes match the remembered body. A different body pauses autosave. */
  async function onRemoteNote() {
    if (id === 'new') return;
    if (saveInFlight) {
      remoteDuringSave = true;
      return;
    }
    let disk: Note;
    try {
      disk = await api.notes.get(id);
    } catch {
      return;
    }
    if (saveInFlight) {
      remoteDuringSave = true;
      return;
    }
    if (disk.content === knownBody) {
      baseHash = disk.contentHash;
      return;
    }
    clearTimeout(saveTimer);
    if (status === 'dirty' || status === 'saving' || externalChange) {
      externalChange = true;
      return;
    }
    applyNote(disk);
    status = 'saved';
    void refreshConflict();
  }

  async function acceptExternal() {
    clearTimeout(saveTimer);
    externalChange = false;
    status = 'saved';
    await load();
  }

  /** Write the editor buffer on top of the body just read from disk. */
  async function keepMine() {
    clearTimeout(saveTimer);
    if (id === 'new') return;
    const body = content;
    try {
      const disk = await api.notes.get(id);
      const n = await api.notes.update(id, { title, content: body, tags, base_hash: disk.contentHash });
      baseHash = n.contentHash;
      knownBody = body;
      updatedAt = n.updated_at;
      chips = n.chips;
      bindings = n.bindings;
      externalChange = false;
      status = 'saved';
    } catch (e) {
      if (hasErrorCode(e, 'conflict_changed')) {
        externalChange = true;
        status = 'dirty';
        return;
      }
      error = formatError(e);
      status = 'failed';
    }
  }

  async function refreshConflict() {
    if (id === 'new') return;
    try {
      const s = await api.sync.status();
      hasConflict = s.conflicts.some((c) => c.note_id === id);
    } catch {
      hasConflict = false;
    }
  }

  /** Flush pending edits first so the "mine" side is the text on screen. */
  async function openConflict() {
    await save();
    sheet = 'conflict';
  }

  onMount(() => {
    const fromSettings = page.url.searchParams.has('conflict');
    load().then(async () => {
      await refreshConflict();
      if (fromSettings && id !== 'new') {
        replaceState(`/notes/${id}`, {});
        await openConflict();
      }
    });
    loadCatalogs();
    const unRemote = onNoteRemote((changedId) => {
      if (changedId !== id) return;
      void onRemoteNote();
    });
    const unTransfer = onAttachmentTransfer((all) => {
      transfers = new Map([...all].filter(([, x]) => x.note_id === id));
      if ([...all.values()].some((x) => x.note_id === id && x.finished)) void loadAttachments();
    });
    return () => {
      unRemote.then((f) => f());
      unTransfer.then((f) => f());
    };
  });

  // Following a wiki link keeps this component; reload when the route param changes.
  let seenParam = page.params.id;
  $effect(() => {
    const next = page.params.id ?? 'new';
    if (next === seenParam) return;
    seenParam = next;
    // The first save rewrites the URL from /new to the real id; nothing to reload then.
    if (next !== untrack(() => id)) void switchNote(next);
  });

  async function switchNote(next: string) {
    await save();
    urls.release();
    urls = new AttachmentUrls(next);
    id = next;
    title = '';
    content = '';
    tags = [];
    chips = [];
    extraBindings = [];
    droppedBindings = [];
    extraFolders = [];
    droppedFolders = [];
    pinned = false;
    archived = false;
    attachments = [];
    thumbs = {};
    wikiQuery = '';
    wikiOpen = false;
    sheet = 'none';
    ready = next === 'new';
    status = 'idle';
    hasConflict = false;
    externalChange = false;
    baseHash = null;
    knownBody = '';
    fetchOffer = [];
    await load();
    await refreshConflict();
  }

  // ── Editor ──

  /** Attachment images render from blob URLs read through the backend. */
  function resolveSrc(src: string): string {
    const prefix = `attachments/${id}/`;
    let decoded = src;
    try { decoded = decodeURIComponent(src); } catch { /* keep raw */ }
    if (!decoded.startsWith(prefix)) return src;
    return thumbs[decoded.slice(prefix.length)] ?? src;
  }

  function setMode(next: EditorMode) {
    if (next === mode) return;
    mode = next;
    linkOpen = false;
    wikiQuery = '';
    wikiOpen = false;
    saveEditorMode(mode);
  }

  function onRichChange(md: string) {
    content = md;
    onEdit();
  }

  /** Format bar tap: TipTap command in Visual, text transform in Markdown. */
  function runAction(action: EditAction) {
    if (mode === 'rich') {
      if (action === 'link') return openLink();
      rich?.runAction(action);
      return;
    }
    const ta = body;
    if (!ta) return;
    const r = applyAction(content, ta.selectionStart, ta.selectionEnd, action);
    content = r.text;
    onEdit();
    requestAnimationFrame(() => {
      ta.focus();
      ta.setSelectionRange(r.selStart, r.selEnd);
    });
  }

  function openLink() {
    linkHref = rich?.linkHref() ?? '';
    linkOpen = true;
    tick().then(() => linkInput?.focus());
  }

  function applyLink(e: SubmitEvent) {
    e.preventDefault();
    rich?.setLink(linkHref.trim());
    linkOpen = false;
  }

  function closeLink() {
    linkOpen = false;
  }

  /** Track an unclosed `[[` before the caret in source mode. */
  function updateWikiMd() {
    const pos = body?.selectionStart ?? content.length;
    const before = content.slice(0, pos);
    const open = unclosedMarkAt(before, WIKI_OPEN, WIKI_CLOSE);
    if (open < 0) {
      wikiOpen = false;
      return;
    }
    wikiStart = open;
    wikiQuery = before.slice(open + WIKI_OPEN.length);
    wikiOpen = true;
  }

  // ── Template placeholders: `{{` in source mode ──
  let tplOpen = $state(false);
  let tplQuery = $state('');
  let tplStart = 0;
  /** Notes in the Templates folder keep `{{name}}`; elsewhere the value is inserted. */
  const isTemplateNote = $derived(
    folderIds.some((fid) => folders.find((f) => f.id === fid)?.name.toLowerCase() === TEMPLATES_FOLDER.toLowerCase()),
  );

  function updateTplMd() {
    const pos = body?.selectionStart ?? content.length;
    const before = content.slice(0, pos);
    const open = unclosedMarkAt(before, TPL_OPEN, TPL_CLOSE);
    if (open < 0) { tplOpen = false; return; }
    tplStart = open;
    tplQuery = before.slice(open + TPL_OPEN.length);
    tplOpen = true;
  }

  function pickPlaceholder(text: string) {
    if (mode === 'rich') {
      rich?.pickPlaceholder(text);
      tplOpen = false;
      return;
    }
    const pos = body?.selectionStart ?? content.length;
    const rest = content.slice(pos);
    const skip = rest.startsWith(TPL_CLOSE) ? TPL_CLOSE.length : 0;
    content = content.slice(0, tplStart) + text + rest.slice(skip);
    tplOpen = false;
    onEdit();
    const caret = tplStart + text.length;
    requestAnimationFrame(() => body?.setSelectionRange(caret, caret));
  }

  function pickWiki(target: string, label?: string | null) {
    if (mode === 'rich') {
      rich?.pickWikiLink(target, label);
      wikiQuery = '';
      wikiOpen = false;
      return;
    }
    const pos = body?.selectionStart ?? content.length;
    const link = wikiMarkup(target, label);
    const rest = content.slice(pos);
    const skip = rest.startsWith(WIKI_CLOSE) ? WIKI_CLOSE.length : 0;
    content = content.slice(0, wikiStart) + link + rest.slice(skip);
    wikiQuery = '';
    wikiOpen = false;
    onEdit();
    const caret = wikiStart + link.length;
    requestAnimationFrame(() => body?.setSelectionRange(caret, caret));
  }

  /** Open the linked note; create it when nothing matches. Entity mentions open the context sheet. */
  async function openWikiLink(target: string) {
    const name = target.trim();
    if (!name) return;
    if (isEntityBinding(name)) { openContext(name); return; }
    try {
      await save();
      let hit = await api.notes.resolveLink(name);
      if (!hit) hit = (await api.notes.create(name)).id;
      await goto(`/notes/${hit}`);
    } catch (e) {
      error = formatError(e);
    }
  }

  function openNote(noteId: string) {
    sheet = 'none';
    void goto(`/notes/${noteId}`);
  }

  // ── Tags, folders, workspaces ──

  async function addTag(name: string, color?: string) {
    // "Работа" is the tag "работа": the new tag and the note get one name.
    const choice = tagChoice(name.trim().replace(/^#/, ''), allTags, tags);
    if (!choice) {
      sheet = 'none';
      return;
    }
    try {
      if (!choice.existing && color) {
        const created = await api.notes.tagCreate(choice.name, color);
        allTags = [...allTags, created];
      }
      tags = [...tags, choice.name];
      onEdit();
      sheet = 'none';
    } catch (e) {
      error = formatError(e);
    }
  }

  function removeTag(name: string) {
    tags = tags.filter((x) => x !== name);
    onEdit();
  }

  async function addFolder(folderId: string) {
    if (folderIds.includes(folderId)) {
      sheet = 'none';
      return;
    }
    if (id === 'new') {
      droppedFolders = droppedFolders.filter((x) => x !== folderId);
      if (folderId !== inheritedFolderId && !extraFolders.includes(folderId)) {
        extraFolders = [...extraFolders, folderId];
      }
      sheet = 'none';
      return;
    }
    busy = true;
    try {
      await api.notes.addFolder(id, folderId);
      await refreshMeta();
      sheet = 'none';
    } catch (e) {
      error = formatError(e);
    } finally {
      busy = false;
    }
  }

  async function removeFolder(folderId: string) {
    if (id === 'new') {
      extraFolders = extraFolders.filter((x) => x !== folderId);
      if (folderId === inheritedFolderId && !droppedFolders.includes(folderId)) {
        droppedFolders = [...droppedFolders, folderId];
      }
      return;
    }
    busy = true;
    try {
      await api.notes.removeFolder(id, folderId);
      await refreshMeta();
    } catch (e) {
      error = formatError(e);
    } finally {
      busy = false;
    }
  }

  /** Links an owner keeps on its side (passwords) go through it; other bindings go straight onto the note. */
  function bindEntity(binding: string): Promise<void> {
    const parsed = parseBinding(binding);
    const def = parsed?.value ? directory.get(parsed.kind) : undefined;
    if (parsed && def?.link) return def.link.add(parsed.value, `note:${id}`);
    return api.notes.addBinding(id, binding);
  }

  async function addBinding(binding: string) {
    if (binding.startsWith('workspace:')) {
      await setWorkspace(binding.slice('workspace:'.length), true);
      sheet = 'none';
      return;
    }
    if (id === 'new') {
      droppedBindings = droppedBindings.filter((b) => b !== binding);
      if (!extraBindings.includes(binding) && !inheritedBindings.includes(binding)) {
        extraBindings = [...extraBindings, binding];
      }
      sheet = 'none';
      return;
    }
    busy = true;
    try {
      await bindEntity(binding);
      await refreshMeta();
      sheet = 'none';
    } catch (e) {
      error = formatError(e);
    } finally {
      busy = false;
    }
  }

  // ── Attachments ──

  /** Native picker; files are streamed into the note by Rust. */
  async function attach(imagesOnly: boolean) {
    try {
      if (!(await ensureSaved())) return;
      const added = await api.attachments.pick(id, imagesOnly);
      await loadAttachments();
      for (const a of added) if (a.is_image) insertAttachment(a);
    } catch (err) {
      error = formatError(err);
    }
  }

  /** Creates the note first when it is not stored yet; false if that failed. */
  async function ensureSaved(): Promise<boolean> {
    if (id !== 'new') return true;
    status = 'dirty';
    await save();
    return id !== 'new';
  }

  async function attachMedia(kind: MediaKind) {
    try {
      if (!(await ensureSaved())) return;
      await api.attachments.pickMedia(id, kind);
      await loadAttachments();
    } catch (err) {
      error = formatError(err);
    }
  }

  /** Stores a finished recording and links it in the body. */
  async function storeCapture(file: CaptureFile) {
    try {
      if (!(await ensureSaved())) return;
      const added = await api.attachments.addBlob(id, file.name, file.blob);
      await loadAttachments();
      insertAttachment(added);
    } catch (err) {
      error = formatError(err);
    }
  }

  function attachmentOf(item: MediaItem): NoteAttachment | undefined {
    return attachments.find((a) => a.name === item.name);
  }

  function insertText(text: string) {
    if (mode === 'rich' && rich) {
      rich.insertMarkdown(text);
      return;
    }
    const pos = body?.selectionStart ?? content.length;
    const before = content.slice(0, pos);
    const sep = before && !before.endsWith('\n') ? '\n' : '';
    content = `${before}${sep}${text}\n${content.slice(pos)}`;
    onEdit();
  }

  function insertAttachment(a: NoteAttachment) {
    const href = markdownDestination(attachmentHref(a.rel_path));
    insertText(a.is_image ? `![${a.name}](${href})` : `[${a.name}](${href})`);
  }

  async function saveAttachment(a: NoteAttachment) {
    try {
      await api.attachments.save(id, a.name);
    } catch (err) {
      error = formatError(err);
    }
  }

  async function removeAttachment(a: NoteAttachment) {
    if (!(await ask({ title: $t('notes_attach_delete_confirm', { name: a.name }) }))) return;
    try {
      await api.attachments.delete(id, a.name);
      await loadAttachments();
    } catch (err) {
      error = formatError(err);
    }
  }

  // ── Save ──

  function onEdit() {
    status = 'dirty';
    clearTimeout(saveTimer);
    if (externalChange) return;
    saveTimer = setTimeout(save, 700);
  }

  async function save() {
    clearTimeout(saveTimer);
    if (externalChange) return;
    if (status !== 'dirty') return saving ?? undefined;
    if (saving) {
      // Let the in-flight write finish, then persist the latest state.
      await saving;
      return save();
    }
    status = 'saving';
    saving = (async () => {
      saveInFlight = true;
      try {
        if (id === 'new') {
          if (!title.trim() && !content.trim()) {
            status = 'idle';
            return;
          }
          const bindingsToApply = draftBindings;
          const foldersToApply = folderIds;
          const n = await api.notes.create(title, content, tags, bindingsToApply);
          id = n.id;
          for (const folderId of foldersToApply) await api.notes.addFolder(id, folderId);
          // Draft links to entities that keep the link on their side (passwords) are completed there
          for (const b of bindingsToApply) {
            const parsed = parseBinding(b);
            const def = parsed?.value ? directory.get(parsed.kind) : undefined;
            if (parsed && def?.link) await def.link.add(parsed.value, `note:${id}`);
          }
          const fresh = await api.notes.get(id);
          urls = new AttachmentUrls(id);
          chips = fresh.chips;
          bindings = fresh.bindings;
          createdAt = fresh.created_at;
          updatedAt = fresh.updated_at;
          baseHash = fresh.contentHash;
          knownBody = fresh.content;
          replaceState(`/notes/${id}`, {});
        } else {
          const body = content;
          const n = await api.notes.update(id, { title, content: body, tags, base_hash: baseHash ?? undefined });
          knownBody = body;
          baseHash = n.contentHash;
          updatedAt = n.updated_at;
          chips = n.chips;
          bindings = n.bindings;
        }
        status = 'saved';
      } catch (e) {
        if (hasErrorCode(e, 'conflict_changed')) {
          externalChange = true;
          status = 'dirty';
          return;
        }
        error = formatError(e);
        status = 'failed';
      } finally {
        saving = null;
        saveInFlight = false;
        if (remoteDuringSave) {
          remoteDuringSave = false;
          void onRemoteNote();
        }
      }
    })();
    return saving;
  }

  async function setFlags(input: { pinned?: boolean; archived?: boolean }) {
    sheet = 'none';
    if (id === 'new') return;
    try {
      const n = await api.notes.update(id, input);
      pinned = n.pinned;
      archived = n.archived;
    } catch (e) {
      error = formatError(e);
    }
  }

  const setWorkspace = (wsId: string, on: boolean) => setBinding(`workspace:${wsId}`, on);

  /** Add or remove any `kind:value` binding; drafts keep it locally until the first save. */
  async function setBinding(binding: string, on: boolean) {
    if (id === 'new') {
      if (on) {
        droppedBindings = droppedBindings.filter((b) => b !== binding);
        if (!extraBindings.includes(binding) && !inheritedBindings.includes(binding)) {
          extraBindings = [...extraBindings, binding];
        }
      } else {
        extraBindings = extraBindings.filter((b) => b !== binding);
        if (inheritedBindings.includes(binding) && !droppedBindings.includes(binding)) {
          droppedBindings = [...droppedBindings, binding];
        }
      }
      return;
    }
    busy = true;
    try {
      if (on) await bindEntity(binding);
      else await api.notes.removeBinding(id, binding);
      await refreshMeta();
    } catch (e) {
      error = formatError(e);
    } finally {
      busy = false;
    }
  }

  async function moveTo(folderId: string | null) {
    busy = true;
    try {
      await api.notes.setFolder(id, folderId);
      const n = await api.notes.get(id);
      chips = n.chips;
      bindings = n.bindings;
      sheet = 'none';
    } catch (e) {
      error = formatError(e);
    } finally {
      busy = false;
    }
  }

  async function remove() {
    sheet = 'none';
    if (id === 'new') return goto('/notes');
    if (!(await ask({ title: $t('notes_delete_confirm', { title: title || $t('notes_untitled') }) }))) return;
    clearTimeout(saveTimer);
    status = 'idle';
    try {
      await api.notes.delete(id);
      goto('/notes', { replaceState: true });
    } catch (e) {
      error = formatError(e);
    }
  }

  function back() {
    void leave('/notes');
  }

  // Flush a pending edit when the user navigates away.
  onDestroy(() => {
    if (status === 'dirty') save();
    urls.release();
  });
</script>

<!-- A tag or folder chip: a tap opens the labels sheet; only its × removes it. -->
{#snippet removable(kind: string, label: string, color: string | undefined, onremove: () => void)}
  <span class="m-chip removable tinted" style:--chip={color}>
    <button type="button" class="chip-body" onclick={() => (sheet = 'labels')}><ChipMark {kind} />{label}</button>
    <button type="button" class="chip-x" onclick={onremove} aria-label={$t('notes_label_remove', { name: label })}><Icon name="x" size={14} /></button>
  </span>
{/snippet}

<div class="editor" style:padding-bottom={kb > 0 ? `max(${kb}px, calc(72px + var(--sab)))` : undefined}>
  <div class="m-header">
    <button class="m-ibtn" onclick={back} aria-label={$t('common_back')}>
      <Icon name="chevron-left" size={24} />
    </button>
    {#if editingTitle}
      <input
        class="crumb-edit"
        bind:value={title}
        oninput={onEdit}
        onblur={() => (editingTitle = false)}
        onkeydown={(e) => e.key === 'Enter' && (editingTitle = false)}
        placeholder={$t('notes_title_placeholder')}
        {@attach (el: HTMLInputElement) => el.focus()}
      />
    {:else}
      <button type="button" class="crumb" onclick={() => (editingTitle = true)}>
        <span class="name">{title.trim() || $t('notes_untitled')}</span>
        <Icon name="pencil" size={14} />
      </button>
    {/if}
    <!-- The save state is a badge, not a button; pin and the menu are header buttons of full size -->
    {#if status !== 'idle'}
      <span
        class="save"
        class:ok={status === 'saved'}
        class:busy={status === 'saving' || status === 'dirty'}
        class:bad={status === 'failed'}
        class:spin={status === 'saving'}
        title={status === 'saved' ? $t('notes_saved') : status === 'failed' ? error : $t('notes_saving')}
      >
        <Icon name={status === 'saved' ? 'check-circle' : status === 'failed' ? 'alert-triangle' : status === 'saving' ? 'loader' : 'circle'} size={16} />
      </span>
    {/if}
    <button type="button" class="m-ibtn" class:on={pinned} disabled={isNew} onclick={() => setFlags({ pinned: !pinned })} aria-label={pinned ? $t('notes_unpin') : $t('notes_pin')}>
      <Icon name="pin" size={20} />
    </button>
    <button type="button" class="m-ibtn" onclick={() => (sheet = 'actions')} aria-label={$t('notes_actions')}>
      <Icon name="more-vertical" size={22} />
    </button>
  </div>
  <div class="meta">
    {#if updatedAt}
      <p class="when">{$t('notes_meta_updated')} · {fmtDateTime(updatedAt, $locale)}</p>
    {/if}
    {#if error}
      <div class="m-error">{error}</div>
    {/if}
    <div class="m-chips tags">
      {#each shownChips as c (c.kind + c.id)}
        {#if c.kind === 'folder'}
          {@render removable('folder', c.label, c.color, () => removeFolder(c.id))}
        {:else if c.kind === 'proxy' || c.kind === 'ssh' || c.kind === 'totp' || c.kind === 'password'}
          <button type="button" class="m-chip tinted" style:--chip={c.color} onclick={() => openContext(`${c.kind}:${c.id}`)}><ChipMark kind={c.kind} />{c.label}</button>
        {:else}
          <span class="m-chip tinted" style:--chip={c.color || (c.kind === 'workspace' ? 'var(--success)' : undefined)}><ChipMark kind={c.kind} />{c.label}</span>
        {/if}
      {/each}
      {#each tags as name (name)}
        {@render removable('tag', name, findTag(allTags, name)?.color, () => removeTag(name))}
      {/each}
      <button class="m-chip add" onclick={() => (sheet = 'labels')} aria-label={$t('notes_tags_add')}>
        <Icon name="plus" size={14} />
      </button>
    </div>
    <NoteFormatBar {mode} disabled={!ready} onaction={runAction} onmode={setMode} />
    {#if linkOpen}
      <form class="link-form" onsubmit={applyLink}>
        <input
          bind:this={linkInput}
          bind:value={linkHref}
          type="text"
          inputmode="url"
          autocapitalize="off"
          autocomplete="off"
          placeholder="https://"
          aria-label={$t('note_tb_link')}
        />
        <button type="submit" class="btn btn-primary">{$t('common_done')}</button>
        <button type="button" class="btn btn-ghost" onclick={closeLink}>{$t('common_cancel')}</button>
      </form>
    {/if}
    {#if externalChange}
      <div class="conflict-banner">
        <Icon name="alert-triangle" size={16} />
        <span>{$t('note_external_change')}</span>
        <button type="button" class="btn btn-ghost" onclick={acceptExternal}>{$t('note_external_accept')}</button>
        <button type="button" class="btn btn-primary" onclick={keepMine}>{$t('note_external_keep')}</button>
      </div>
    {/if}
    {#if hasConflict}
      <div class="conflict-banner">
        <Icon name="alert-triangle" size={16} />
        <span>{$t('note_sync_conflict')}</span>
        <button type="button" class="btn btn-primary" onclick={openConflict}>{$t('note_sync_conflict_resolve')}</button>
      </div>
    {/if}
    {#if fetchOffer.length}
      <div class="fetch-card">
        <span class="m-doc"><Icon name="download" size={16} /></span>
        <span class="fetch-text">{$t('note_att_fetch_prompt', { n: String(fetchOffer.length), size: fmtSize(fetchOfferBytes) })}</span>
        <div class="fetch-actions">
          <button type="button" class="btn btn-ghost" onclick={() => (fetchOffer = [])}>{$t('note_att_fetch_later')}</button>
          <button type="button" class="btn btn-primary" onclick={fetchOffered}>{$t('note_att_fetch_yes')}</button>
        </div>
      </div>
    {/if}
  </div>

  <div class="scroll" bind:this={scrollEl}>
  {#if !ready}
    <div class="body"></div>
  {:else if mode === 'rich'}
    <NoteRichEditor
      bind:this={rich}
      autofocus={isNew}
      {content}
      {resolveSrc}
      placeholder={$t('notes_body_placeholder')}
      onchange={onRichChange}
      onwiki={(q) => { wikiOpen = q !== null; if (q !== null) wikiQuery = q; }}
      onwikilink={openWikiLink}
      ontpl={(q) => { tplOpen = q !== null; if (q !== null) tplQuery = q; }}
    />
  {:else}
    <textarea
      class="body"
      bind:this={body}
      {@attach (el: HTMLTextAreaElement) => { if (isNew) el.focus(); }}
      bind:value={content}
      oninput={() => { onEdit(); updateWikiMd(); updateTplMd(); }}
      onclick={() => { updateWikiMd(); updateTplMd(); }}
      onkeyup={() => { updateWikiMd(); updateTplMd(); }}
      placeholder={$t('notes_body_placeholder')}
    ></textarea>
  {/if}
  </div>

</div>

<div class="toolbar" class:hide={wikiOpen || tplOpen}>
  <button type="button" onclick={() => (sheet = 'attach')} aria-label={$t('notes_attach')}>
    <Icon name="paperclip" size={22} />
  </button>
  <MediaCapture
    items={mediaItems}
    locale={$locale}
    oncapture={storeCapture}
    onpick={attachMedia}
    oninsert={(i) => { const a = attachmentOf(i); if (a) insertAttachment(a); }}
    onremove={(i) => { const a = attachmentOf(i); if (a) removeAttachment(a); }}
    onsave={(i) => { const a = attachmentOf(i); if (a) saveAttachment(a); }}
    resolveSrc={(i) => urls.get(i.name)}
  />
  <button type="button" disabled={isNew} onclick={() => (sheet = 'links')} aria-label={$t('notes_links')}>
    <Icon name="link" size={22} />
  </button>
  {#if showContext}
    <button type="button" class="ctx-btn" onclick={() => openContext()} aria-label={$t('notes_context')}>
      <Icon name="layers" size={22} />
      {#if contextCount > 0}<span class="ctx-count">{contextCount}</span>{/if}
    </button>
  {/if}
</div>

<NoteActionsSheet
  open={sheet === 'actions'}
  noteId={id}
  {pinned}
  {archived}
  onclose={() => (sheet = 'none')}
  onhistory={() => (sheet = 'history')}
  onpin={() => setFlags({ pinned: !pinned })}
  onarchive={() => setFlags({ archived: !archived })}
  onmove={() => (sheet = 'move')}
  onworkspace={workspaces.length > 0 ? () => (sheet = 'workspace') : undefined}
  ondelete={remove}
/>
<NoteLinksSheet
  open={sheet === 'links'}
  noteId={id}
  onclose={() => (sheet = 'none')}
  onopen={openNote}
  oncreate={(target) => { sheet = 'none'; void openWikiLink(target); }}
/>
<NoteContextSheet
  open={sheet === 'context'}
  noteId={id}
  bindings={entityBindings}
  {mentions}
  focus={contextFocus}
  {busy}
  onclose={() => (sheet = 'none')}
  onopen={openNote}
  onbind={(b) => addBinding(b)}
  onunbind={(b) => setBinding(b, false)}
/>
<NoteHistorySheet
  open={sheet === 'history'}
  noteId={id}
  {urls}
  onclose={() => (sheet = 'none')}
  onrestored={(n) => { applyNote(n); status = 'saved'; }}
/>
<MoveFolderSheet
  open={sheet === 'move'}
  {folders}
  current={folderChip?.id ?? null}
  {busy}
  onclose={() => (sheet = 'none')}
  onmove={moveTo}
/>
<WorkspaceSheet
  open={sheet === 'workspace'}
  {workspaces}
  current={workspaceIds}
  {busy}
  onclose={() => (sheet = 'none')}
  ontoggle={setWorkspace}
/>
<NoteLabelSheet
  open={sheet === 'labels'}
  tags={allTags}
  selectedTags={tags}
  {folders}
  {folderIds}
  {workspaces}
  {profiles}
  bindings={activeBindings}
  {busy}
  onclose={() => (sheet = 'none')}
  onaddTag={addTag}
  onaddFolder={addFolder}
  onaddBinding={addBinding}
/>
<NoteAttachSheet
  open={sheet === 'attach'}
  {attachments}
  {thumbs}
  {createdAt}
  {updatedAt}
  folderLabel={folderChip?.label ?? ''}
  {tags}
  onclose={() => (sheet = 'none')}
  onfile={() => attach(false)}
  onimage={() => attach(true)}
  {transfers}
  oncancel={(name) => api.sync.attachmentCancel(id, name)}
  oninsert={(a) => { insertAttachment(a); sheet = 'none'; }}
  onremove={removeAttachment}
  onfetch={fetchAttachment}
  onsave={saveAttachment}
/>
<NoteConflictSheet
  open={sheet === 'conflict'}
  noteId={id}
  onclose={() => (sheet = 'none')}
  onresolved={async () => { await load(); await refreshConflict(); }}
/>
<WikiLinkSheet
  open={wikiOpen}
  seed={wikiQuery}
  notes={allNotes}
  excludeId={id}
  onclose={() => { wikiOpen = false; wikiQuery = ''; }}
  onpick={pickWiki}
/>
<PlaceholderSheet
  open={tplOpen && !wikiOpen}
  query={tplQuery}
  isTemplate={isTemplateNote}
  {title}
  bindings={isNew ? draftBindings : bindings}
  onclose={() => { tplOpen = false; tplQuery = ''; }}
  onpick={pickPlaceholder}
/>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    min-height: 0;
    padding: 0 0 calc(72px + var(--sab));
    background: var(--bg);
    animation: vfade var(--dur-base) ease-out;
  }
  .m-header {
    flex-shrink: 0;
    position: relative;
    top: auto;
    margin: 0;
  }
  .meta {
    flex-shrink: 0;
    padding: var(--sp-2) var(--sp-4) var(--sp-2);
    background: var(--bg);
  }
  .conflict-banner {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin-top: var(--sp-2);
    padding: var(--sp-2) var(--sp-3);
    border: 1px solid var(--warn-border, var(--border));
    border-radius: var(--m-radius, 12px);
    background: var(--warn-bg, var(--surface-2));
    color: var(--warn-text, var(--text));
    font-size: 13px;
  }
  .conflict-banner span { flex: 1; min-width: 0; }
  .conflict-banner .btn { min-height: 36px; padding: 0 var(--sp-3); flex-shrink: 0; }
  .fetch-card {
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: center;
    gap: var(--sp-2) var(--sp-3);
    margin-top: var(--sp-2);
    padding: var(--sp-3);
    border: 1px solid var(--accent-border, var(--m-card-border));
    border-radius: var(--m-radius, 12px);
    background: var(--accent-bg);
    font-size: 13px;
    color: var(--text);
  }
  .fetch-card .m-doc { color: var(--accent); }
  .fetch-text { min-width: 0; line-height: 1.35; }
  .fetch-actions { grid-column: 1 / -1; display: flex; justify-content: flex-end; gap: var(--sp-2); }
  .fetch-actions .btn { min-height: 36px; padding: 0 var(--sp-3); }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    -webkit-overflow-scrolling: touch;
    padding: var(--sp-3) var(--sp-4);
  }
  .crumb {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    text-align: left;
  }
  .crumb .name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 17px; font-weight: 700; }
  .crumb :global(svg) { flex-shrink: 0; color: var(--text-3); }
  .crumb-edit {
    flex: 1;
    min-width: 0;
    min-height: 40px;
    padding: 0 4px;
    border: 0;
    border-radius: 8px;
    background: var(--m-field);
    font-size: 17px;
    font-weight: 700;
  }
  .crumb-edit:focus { box-shadow: none; }
  .m-header .m-ibtn { flex-shrink: 0; }
  .m-header .m-ibtn:disabled { opacity: 0.35; }
  .save { display: inline-flex; align-items: center; flex-shrink: 0; padding: 0 var(--sp-1); color: var(--text-3); }
  .save.ok { color: var(--success-text); }
  .save.busy { color: var(--warn-text); }
  .save.bad { color: var(--danger-text); }
  .save.spin :global(svg) { animation: spin 1s linear infinite; }
  .when { margin: 0 0 var(--sp-2); font-size: var(--fs-xs); color: var(--text-3); }
  .removable { gap: 0; padding: 0 0 0 12px; }
  .chip-body, .chip-x {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-height: 28px;
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
  }
  .chip-x { justify-content: center; width: 30px; opacity: var(--chip-fade); }
  .link-form { display: flex; align-items: center; gap: var(--sp-2); margin-top: var(--sp-2); }
  .link-form input { flex: 1; min-width: 0; min-height: 36px; padding: 0 var(--sp-3); border: 1px solid var(--border); border-radius: 8px; background: var(--m-field); }
  .link-form .btn { min-height: 36px; padding: 0 var(--sp-3); flex-shrink: 0; }
  .body {
    border: 0;
    background: transparent;
    padding: var(--sp-2) 0;
    resize: none;
    min-height: 200px;
    field-sizing: content;
    font-family: var(--font-mono);
    font-size: 15px;
    line-height: 1.55;
  }
  .body:focus { box-shadow: none; }
  .editor :global(.rich) { flex: none; }
  .editor :global(.ProseMirror) { overflow: visible; min-height: 200px; font-size: 16px; line-height: 1.55; }

  .tags { margin-bottom: var(--sp-2); }

  .toolbar {
    position: fixed;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 20;
    display: flex;
    justify-content: space-around;
    align-items: center;
    height: calc(56px + var(--sab));
    padding-bottom: var(--sab);
    background: var(--m-nav);
    border-top: 1px solid var(--border);
  }
  .toolbar button {
    width: 48px;
    height: 48px;
    padding: 0;
    border: 0;
    border-radius: 12px;
    background: transparent;
    color: var(--text-2);
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .toolbar button.on { color: var(--accent); }
  .toolbar button:active { background: var(--m-seg); }
  .toolbar button:disabled { opacity: 0.35; }
  .toolbar.hide { display: none; }
  .ctx-btn { position: relative; }
  .ctx-count {
    position: absolute; top: 6px; right: 6px;
    min-width: 16px; height: 16px; padding: 0 4px;
    border-radius: 8px; background: var(--accent); color: #fff;
    font-size: 10px; font-weight: 700; line-height: 16px; text-align: center;
  }
</style>
