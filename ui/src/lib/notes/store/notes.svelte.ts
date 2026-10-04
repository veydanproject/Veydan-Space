// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { api } from '$lib/notes/api';
import { hasErrorCode } from '$lib/core/utils';
import type { ActiveFilter } from '$lib/notes/filter';
import type {
  Note,
  NoteCreateInput,
  NoteFilter,
  NoteFolder,
  NoteListItem,
  NoteSmartView,
  NoteTag,
  NoteUpdateInput,
  SaveStatus,
  SmartViewInput,
} from '$lib/notes/types';

const AUTOSAVE_DELAY_MS = 3000;
const DRAFT_INTERVAL_MS = 1000;

export type NotesUiRequest =
  | { kind: 'create' }
  | { kind: 'search' }
  | { kind: 'insertLink' }
  | { kind: 'filter'; filter: ActiveFilter };

class NotesStore {
  /** All live notes (active + archived) for sidebar counts. Trashed notes live in `trash`. */
  list = $state<NoteListItem[]>([]);
  /** Notes matching `filter` (backend-filtered) or the current search */
  view = $state<NoteListItem[]>([]);
  trash = $state<NoteListItem[]>([]);
  allTags = $state<NoteTag[]>([]);
  folders = $state<NoteFolder[]>([]);
  smartViews = $state<NoteSmartView[]>([]);
  loading = $state(false);
  loaded = $state(false);
  private _promise: Promise<void> | null = null;

  // Editor state
  activeNoteId = $state<string | null>(null);
  /** Note id requested from the browser extension popup or the command palette */
  openRequestId = $state<string | null>(null);
  /** UI request from the command palette, consumed by the notes page / editor */
  uiRequest = $state<NotesUiRequest | null>(null);
  activeNote = $state<Note | null>(null);
  saveStatus = $state<SaveStatus>('saved');
  externalChange = $state(false);
  /** Hash of the body last loaded or saved. Sent back so a stale buffer cannot overwrite the file. */
  private _baseHash: string | null = null;
  /** Body that `_baseHash` belongs to. Watcher echoes match this; a different body is external. */
  private _knownBody: string | null = null;
  private _saveInFlight = false;
  private _remoteWhileSaving = false;

  // Filter/navigation state
  filter = $state<NoteFilter>({ archived: false });
  searchQuery = $state('');

  // Private autosave/draft state
  private _autosaveTimer: ReturnType<typeof setTimeout> | null = null;
  private _draftInterval: ReturnType<typeof setInterval> | null = null;
  private _pendingContent: string | null = null;
  private _pendingTitle: string | null = null;
  private _unlisten: (() => void) | null = null;

  async ensureLoaded() {
    if (this.loaded) return;
    if (this._promise) return this._promise;
    this._promise = this._load().finally(() => { this._promise = null; });
    return this._promise;
  }

  private async _load() {
    this.loading = true;
    try {
      await Promise.all([this.refresh(), this.refreshTags(), this.refreshFolders(), this.refreshSmartViews()]);
      this.loaded = true;
    } finally {
      this.loading = false;
    }
  }

  /** Reload the full list and the filtered view (unless a search is active) */
  async refresh() {
    const [list, view] = await Promise.all([
      api.notes.list({}),
      this.searchQuery ? api.notes.search(this.searchQuery, this.filter) : api.notes.list(this.filter),
    ]);
    this.list = list;
    this.view = view;
  }

  /** Apply a new filter and reload the view */
  async setFilter(filter: NoteFilter) {
    this.filter = filter;
    this.searchQuery = '';
    this.view = await api.notes.list(filter);
  }

  async search(query: string) {
    this.searchQuery = query.trim();
    this.view = this.searchQuery
      ? await api.notes.search(this.searchQuery, this.filter)
      : await api.notes.list(this.filter);
  }

  async refreshSmartViews() {
    this.smartViews = await api.notes.smartViewList();
  }

  async createSmartView(input: SmartViewInput): Promise<NoteSmartView> {
    const view = await api.notes.smartViewCreate(input);
    await this.refreshSmartViews();
    return view;
  }

  async updateSmartView(id: string, input: SmartViewInput): Promise<NoteSmartView> {
    const view = await api.notes.smartViewUpdate(id, input);
    await this.refreshSmartViews();
    return view;
  }

  async deleteSmartView(id: string): Promise<void> {
    await api.notes.smartViewDelete(id);
    await this.refreshSmartViews();
  }

  async refreshTrash() {
    this.trash = await api.notes.list({ deleted: true });
  }

  async refreshTags() {
    this.allTags = await api.notes.tagList();
  }

  async refreshFolders() {
    this.folders = await api.notes.folderList();
  }

  async createTag(name: string, color?: string): Promise<NoteTag> {
    const tag = await api.notes.tagCreate(name, color);
    await this.refreshTags();
    return tag;
  }

  async deleteTag(id: string): Promise<void> {
    await api.notes.tagDelete(id);
    await Promise.all([this.refresh(), this.refreshTags()]);
  }

  async updateTag(id: string, name?: string, color?: string): Promise<NoteTag> {
    const tag = await api.notes.tagUpdate(id, name, color);
    await Promise.all([this.refresh(), this.refreshTags()]);
    return tag;
  }

  async createFolder(name: string, parent_id?: string, color?: string): Promise<NoteFolder> {
    const folder = await api.notes.folderCreate(name, parent_id, color);
    await this.refreshFolders();
    return folder;
  }

  async updateFolder(id: string, name?: string, color?: string): Promise<NoteFolder> {
    const folder = await api.notes.folderUpdate(id, name, color);
    await this.refreshFolders();
    return folder;
  }

  async deleteFolder(id: string): Promise<void> {
    await api.notes.folderDelete(id);
    await Promise.all([this.refresh(), this.refreshFolders()]);
  }

  async addNoteBinding(noteId: string, binding: string): Promise<void> {
    await api.notes.noteAddBinding(noteId, binding);
    await this.refresh();
    if (this.activeNoteId === noteId && this.activeNote && !this.activeNote.bindings.includes(binding)) {
      this.activeNote = { ...this.activeNote, bindings: [...this.activeNote.bindings, binding] };
    }
  }

  async removeNoteBinding(noteId: string, binding: string): Promise<void> {
    await api.notes.noteRemoveBinding(noteId, binding);
    await this.refresh();
    if (this.activeNoteId === noteId && this.activeNote) {
      this.activeNote = { ...this.activeNote, bindings: this.activeNote.bindings.filter(b => b !== binding) };
    }
  }

  async addNoteFolder(noteId: string, folderId: string): Promise<void> {
    await api.notes.noteAddFolder(noteId, folderId);
    await this.refresh();
  }

  async removeNoteFolder(noteId: string, folderId: string): Promise<void> {
    await api.notes.noteRemoveFolder(noteId, folderId);
    await this.refresh();
  }

  /** Open a note in the editor */
  async openNote(id: string) {
    // Allow retry if note was selected but failed to load (activeNote is null)
    if (this.activeNoteId === id && this.activeNote !== null) return;

    // Capture pending save for the OLD note before switching
    if (this._autosaveTimer) { clearTimeout(this._autosaveTimer); this._autosaveTimer = null; }
    const pendingContent = this._pendingContent;
    const pendingTitle = this._pendingTitle;
    const prevId = this.activeNoteId;
    const prevHash = this._baseHash;
    const blocked = this.externalChange;
    this._pendingContent = null;
    this._pendingTitle = null;
    this._stopDraftInterval();

    // Update active ID immediately so the list highlights the new note right away
    this.activeNoteId = id;
    this.externalChange = false;
    this.saveStatus = 'saved';
    this._baseHash = null;
    this._knownBody = null;

    // Save previous note in background (don't block the switch). A conflict leaves the file as-is.
    if (!blocked && (pendingContent !== null || pendingTitle !== null) && prevId) {
      const input: NoteUpdateInput = {};
      if (pendingContent !== null) {
        input.content = pendingContent;
        if (prevHash) input.base_hash = prevHash;
      }
      if (pendingTitle !== null) input.title = pendingTitle;
      void api.notes.update(prevId, input).then((u) => this._patchListItem(u)).catch(() => {});
    }

    // Load new note content
    try {
      const loaded = await api.notes.get(id);
      // Guard against race: another note may have been opened while loading
      if (this.activeNoteId === id) {
        this.activeNote = loaded;
        this._rememberBody(loaded);
      }
    } catch (e) {
      console.error('[notes] Failed to load note:', id, e);
      // Only clear activeNote if watcher hasn't already loaded it
      if (this.activeNoteId === id && this.activeNote === null) {
        // activeNote already null — nothing to do, state stays as "failed to load"
      }
    }
  }

  /** Close editor, flush any pending save */
  async closeNote() {
    await this._flushAutosave();
    this._stopDraftInterval();
    this.activeNoteId = null;
    this.activeNote = null;
    this._pendingContent = null;
    this._baseHash = null;
    this._knownBody = null;
    this.externalChange = false;
    this.saveStatus = 'saved';
  }

  /**
   * Called by NoteEditor on every keystroke.
   * Schedules autosave debounce + starts draft interval.
   */
  onContentChange(content: string) {
    if (!this.activeNoteId) return;
    this._pendingContent = content;
    this.saveStatus = 'unsaved';
    if (this.externalChange) return;
    this._scheduleAutosave();
    this._startDraftInterval();
  }

  onTitleChange(title: string) {
    if (!this.activeNoteId || !this.activeNote) return;
    this._pendingTitle = title;
    this.saveStatus = 'unsaved';
    // Mutating the note retriggers the editor effect and would replace the body.
    if (this.externalChange) return;
    this.activeNote.title = title;
    this._scheduleAutosave();
  }

  private _scheduleAutosave() {
    if (this._autosaveTimer) clearTimeout(this._autosaveTimer);
    this._autosaveTimer = setTimeout(() => {
      this._autosaveTimer = null;
      void this._doAutosave();
    }, AUTOSAVE_DELAY_MS);
  }

  private async _doAutosave() {
    if (this.externalChange) return;
    if (!this.activeNoteId || !this.activeNote) return;
    const content = this._pendingContent;
    const title = this._pendingTitle;
    if (content === null && title === null) return;
    const noteId = this.activeNoteId;
    const base = this._baseHash;
    this.saveStatus = 'saving';
    // Clear pending + stop the draft ticker BEFORE awaiting. The backend save
    // deletes the .draft file; if the 1s interval fired during the await it would
    // re-create it afterwards, leaving a stale draft and a false "draft found" banner.
    this._pendingContent = null;
    this._pendingTitle = null;
    this._stopDraftInterval();
    this._saveInFlight = true;
    try {
      const input: NoteUpdateInput = {};
      if (content !== null) {
        input.content = content;
        if (base) input.base_hash = base;
      }
      if (title !== null) input.title = title;
      const updated = await api.notes.update(noteId, input);
      // Another note may have been opened while awaiting — don't clobber it.
      if (this.activeNoteId === noteId) {
        this.activeNote = updated;
        this._rememberBody(updated);
        this.saveStatus = 'saved';
      }
      this._patchListItem(updated);
    } catch (e) {
      if (this.activeNoteId !== noteId) return;
      if (content !== null && this._pendingContent === null) this._pendingContent = content;
      if (title !== null && this._pendingTitle === null) this._pendingTitle = title;
      if (hasErrorCode(e, 'conflict_changed')) {
        this.externalChange = true;
        this.saveStatus = 'external';
        return;
      }
      this.saveStatus = 'failed';
    } finally {
      this._saveInFlight = false;
      if (this._remoteWhileSaving) {
        this._remoteWhileSaving = false;
        const id = this.activeNoteId;
        if (id) void this._handleExternal(id);
      }
    }
  }

  /** Remember the body a later watcher event must match to count as our own save. */
  private _rememberBody(note: Note) {
    this._baseHash = note.content_hash;
    this._knownBody = note.content;
  }

  /** Patch a saved note into `list`/`view` in place, keeping list-only fields (e.g. preview). */
  private _patchListItem(updated: Note) {
    const patch = (arr: NoteListItem[]) => {
      const idx = arr.findIndex((n) => n.id === updated.id);
      if (idx < 0) return;
      arr[idx] = {
        ...arr[idx],
        title: updated.title,
        updated_at: updated.updated_at,
        pinned: updated.pinned,
        archived: updated.archived,
        tags: updated.tags,
        has_draft: updated.has_draft,
        preview: (updated as unknown as NoteListItem).preview ?? arr[idx].preview,
      };
    };
    patch(this.list);
    patch(this.view);
  }

  /** Explicit save (Ctrl+S) */
  async save() {
    await this._flushAutosave();
  }

  /** Force-save immediately (call before panel close / note switch) */
  private async _flushAutosave() {
    if (this._autosaveTimer) {
      clearTimeout(this._autosaveTimer);
      this._autosaveTimer = null;
    }
    if ((this._pendingContent !== null || this._pendingTitle !== null) && this.activeNoteId) {
      await this._doAutosave();
    }
  }

  private _startDraftInterval() {
    if (this._draftInterval) return;
    this._draftInterval = setInterval(() => {
      if (this._pendingContent !== null && this.activeNoteId) {
        void api.notes.draftSave(this.activeNoteId, this._pendingContent).catch(() => {});
      }
    }, DRAFT_INTERVAL_MS);
  }

  private _stopDraftInterval() {
    if (this._draftInterval) {
      clearInterval(this._draftInterval);
      this._draftInterval = null;
    }
  }

  /** Listen to external file changes (file watcher events from Rust) */
  async startWatcher() {
    if (this._unlisten) return;
    const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
    if (!isTauri) return;
    const { listen } = await import('@tauri-apps/api/event');
    const unlisten = await listen<string>('notes://external-change', (event) => {
      void this._handleExternal(event.payload);
    });
    // Browser extension asked to show a note; the page resets its filter and opens it
    const unlistenOpen = await listen<string>('notes://open', (event) => {
      this.openRequestId = event.payload;
    });
    this._unlisten = () => {
      unlisten();
      unlistenOpen();
    };
  }

  /** Own save echoes match the remembered body. Anything else pauses autosave. */
  private async _handleExternal(changedId: string) {
    if (changedId !== this.activeNoteId) {
      void this.refresh();
      return;
    }
    if (this._saveInFlight) {
      this._remoteWhileSaving = true;
      void this.refresh();
      return;
    }
    let disk: Note | null = null;
    try {
      disk = await api.notes.get(changedId);
    } catch {
      disk = null;
    }
    if (this.activeNoteId !== changedId) return;
    if (this._saveInFlight) {
      this._remoteWhileSaving = true;
      return;
    }
    const diskBody = disk?.content ?? '';
    const sameBody = diskBody === (this._knownBody ?? '');
    if (disk && sameBody) {
      this._baseHash = disk.content_hash;
      void this.refresh();
      return;
    }
    if (this._autosaveTimer) {
      clearTimeout(this._autosaveTimer);
      this._autosaveTimer = null;
    }
    if (this._pendingContent !== null || this._pendingTitle !== null) {
      this.externalChange = true;
      this.saveStatus = 'external';
    } else if (disk) {
      this.activeNote = disk;
      this._rememberBody(disk);
      this.externalChange = false;
      this.saveStatus = 'saved';
    } else {
      this.externalChange = true;
      this.saveStatus = 'external';
    }
    void this.refresh();
  }

  stopWatcher() {
    if (this._unlisten) {
      this._unlisten();
      this._unlisten = null;
    }
  }

  async createNote(input: NoteCreateInput): Promise<Note> {
    const note = await api.notes.create(input);
    await this.refresh();
    return note;
  }

  /** Soft delete: move to trash */
  async deleteNote(id: string) {
    await api.notes.delete(id);
    await this._afterStatusChange(id);
  }

  /** Hard delete: remove file + DB row */
  async deleteForever(id: string) {
    await api.notes.delete(id, true);
    await this._afterStatusChange(id);
  }

  /** Move a whole list to the trash. */
  async deleteMany(ids: string[]) {
    if (!ids.length) return;
    await api.notes.deleteMany(ids);
    if (this.activeNoteId && ids.includes(this.activeNoteId)) this._clearEditorState();
    await Promise.all([this.refresh(), this.refreshTrash()]);
  }

  /** Hard-delete everything in the trash. */
  async emptyTrash() {
    await api.notes.emptyTrash();
    if (this.activeNoteId && this.trash.some((n) => n.id === this.activeNoteId)) this._clearEditorState();
    await Promise.all([this.refresh(), this.refreshTrash()]);
  }

  async archiveNote(id: string) {
    await api.notes.archive(id);
    await this._afterStatusChange(id);
  }

  /** Unarchive or restore from trash */
  async restoreNote(id: string) {
    await api.notes.restore(id);
    await this._afterStatusChange(id);
  }

  /** Note left or entered archive/trash: close it if open, reload list, view and trash. */
  private async _afterStatusChange(id: string) {
    if (this.activeNoteId === id) this._clearEditorState();
    await Promise.all([this.refresh(), this.refreshTrash()]);
  }

  async togglePin(id: string) {
    const note = this.list.find((n) => n.id === id);
    if (!note) return;
    const updated = await api.notes.update(id, { pinned: !note.pinned });
    if (this.activeNoteId === id) this.activeNote = updated;
    await this.refresh();
  }

  async setTags(id: string, tagNames: string[]) {
    await api.notes.setTags(id, tagNames);
    await Promise.all([this.refresh(), this.refreshTags()]);
    if (this.activeNoteId === id && this.activeNote) {
      const listItem = this.list.find(n => n.id === id);
      if (listItem) this.activeNote = { ...this.activeNote, tags: listItem.tags };
    }
  }

  async acceptExternalChange() {
    if (!this.activeNoteId) return;
    if (this._autosaveTimer) { clearTimeout(this._autosaveTimer); this._autosaveTimer = null; }
    try {
      const loaded = await api.notes.get(this.activeNoteId);
      if (this.activeNoteId !== loaded.id) return;
      this.activeNote = loaded;
      this._rememberBody(loaded);
      this.externalChange = false;
      this.saveStatus = 'saved';
      this._pendingContent = null;
      this._pendingTitle = null;
    } catch {}
  }

  /** Write the editor buffer on top of the body just read from disk. */
  async discardExternalChange() {
    if (!this.activeNoteId) return;
    const noteId = this.activeNoteId;
    const content = this._pendingContent;
    const title = this._pendingTitle;
    if (this._autosaveTimer) { clearTimeout(this._autosaveTimer); this._autosaveTimer = null; }
    try {
      const disk = await api.notes.get(noteId);
      if (this.activeNoteId !== noteId) return;
      if (content === null && title === null) {
        this.activeNote = disk;
        this._rememberBody(disk);
        this.externalChange = false;
        this.saveStatus = 'saved';
        return;
      }
      const input: NoteUpdateInput = {};
      if (content !== null) {
        input.content = content;
        if (disk.content_hash) input.base_hash = disk.content_hash;
      }
      if (title !== null) input.title = title;
      const updated = await api.notes.update(noteId, input);
      if (this.activeNoteId !== noteId) return;
      this.activeNote = updated;
      this._rememberBody(updated);
      this._pendingContent = null;
      this._pendingTitle = null;
      this.externalChange = false;
      this.saveStatus = 'saved';
      this._patchListItem(updated);
    } catch (e) {
      if (this.activeNoteId !== noteId) return;
      if (hasErrorCode(e, 'conflict_changed')) {
        this.externalChange = true;
        this.saveStatus = 'external';
        return;
      }
      this.saveStatus = 'failed';
    }
  }

  async recoverDraft(id: string) {
    const draft = await api.notes.draftGet(id);
    if (this.activeNoteId !== id || !this.activeNote) return;
    if (draft === null) {
      // Stale flag — the draft file is already gone. Just clear the banner.
      this.activeNote = { ...this.activeNote, has_draft: false };
      return;
    }
    // Load the recovered draft into the editor as unsaved content, then let
    // autosave persist it to the note file (which also removes the .draft).
    this.activeNote = { ...this.activeNote, content: draft, has_draft: false };
    this._pendingContent = draft;
    this.saveStatus = 'unsaved';
    this._scheduleAutosave();
    this._startDraftInterval();
  }

  async discardDraft(id: string) {
    await api.notes.draftDiscard(id);
    if (this.activeNote && this.activeNote.id === id) {
      this.activeNote = { ...this.activeNote, has_draft: false };
    }
  }

  private _clearEditorState() {
    this._stopDraftInterval();
    if (this._autosaveTimer) {
      clearTimeout(this._autosaveTimer);
      this._autosaveTimer = null;
    }
    this.activeNoteId = null;
    this.activeNote = null;
    this._pendingContent = null;
    this._pendingTitle = null;
    this._baseHash = null;
    this._knownBody = null;
    this.saveStatus = 'saved';
    this.externalChange = false;
  }

  /** Filtered notes for display (computed from list + searchQuery) */
  get filtered(): NoteListItem[] {
    if (!this.searchQuery.trim()) return this.list;
    const q = this.searchQuery.toLowerCase();
    return this.list.filter(
      (n) =>
        n.title.toLowerCase().includes(q) ||
        n.tags.some((t) => t.name.toLowerCase().includes(q))
    );
  }
}

export const notesStore = new NotesStore();
