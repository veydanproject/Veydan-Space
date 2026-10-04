// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Commands of the notes module, and the notes' logic under the `sync_` prefix.

import { api as core, call, mockCommand } from '$lib/core/api';
import type {
  NoteTag,
  NoteFolder,
  NoteListItem,
  Note,
  NoteAttachment,
  OrphanAttachment,
  NoteCreateInput,
  NoteUpdateInput,
  NoteFilter,
  NoteSmartView,
  SmartViewInput,
  NoteNav,
  NoteLinks,
  BindingSummary,
  NoteSyncInfo,
  NoteHistoryEntry,
  MergeResult,
  ConflictView,
  DiffResult,
  HistoryFilter,
} from '$lib/notes/types';
import type { SyncStatus } from '$lib/core/api';

function base64ToBytes(s: string): Uint8Array<ArrayBuffer> {
  const bin = atob(s);
  const out = new Uint8Array(new ArrayBuffer(bin.length));
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

export const api = {
  ...core,
  notes: {
    list: (filter?: NoteFilter) => call<NoteListItem[]>('note_list', { filter: filter ?? {} }),
    nav: () => call<NoteNav>('note_nav'),
    get: (id: string) => call<Note>('note_get', { id }),
    create: (input: NoteCreateInput) => call<Note>('note_create', { input }),
    update: (id: string, input: NoteUpdateInput) => call<Note>('note_update', { id, input }),
    delete: (id: string, hard?: boolean) => call<void>('note_delete', { id, hard }),
    deleteMany: (ids: string[]) => call<void>('note_delete_many', { ids }),
    emptyTrash: () => call<number>('note_trash_empty'),
    archive: (id: string) => call<void>('note_archive', { id }),
    restore: (id: string) => call<void>('note_restore', { id }),
    setTags: (id: string, tagNames: string[]) => call<void>('note_set_tags', { id, tagNames }),
    search: (query: string, filter?: NoteFilter) =>
      call<NoteListItem[]>('note_search', { query, filter: filter ?? {} }),
    sync: () => call<void>('note_sync'),
    reindex: () => call<void>('note_reindex'),
    openFolder: () => call<void>('note_open_folder'),
    openExternal: (id: string) => call<void>('note_open_external', { id }),
    draftSave: (id: string, content: string) => call<void>('note_draft_save', { id, content }),
    draftGet: (id: string) => call<string | null>('note_draft_get', { id }),
    draftDiscard: (id: string) => call<void>('note_draft_discard', { id }),
    tagList: () => call<NoteTag[]>('note_tag_list'),
    tagCreate: (name: string, color?: string) => call<NoteTag>('note_tag_create', { name, color }),
    tagDelete: (id: string) => call<void>('note_tag_delete', { id }),
    tagUpdate: (id: string, name?: string, color?: string) => call<NoteTag>('note_tag_update', { id, name, color }),
    folderList: () => call<NoteFolder[]>('note_folder_list'),
    folderCreate: (name: string, parent_id?: string, color?: string) => call<NoteFolder>('note_folder_create', { name, parentId: parent_id, color }),
    folderUpdate: (id: string, name?: string, color?: string) => call<NoteFolder>('note_folder_update', { id, name, color }),
    folderDelete: (id: string) => call<void>('note_folder_delete', { id }),
    noteAddFolder: (noteId: string, folderId: string) => call<void>('note_add_folder', { noteId, folderId }),
    noteRemoveFolder: (noteId: string, folderId: string) => call<void>('note_remove_folder', { noteId, folderId }),
    /** Replace every folder membership with one folder (`null` = none). */
    noteSetFolder: (noteId: string, folderId: string | null) => call<void>('note_set_folder', { noteId, folderId }),
    noteAddBinding: (noteId: string, binding: string) => call<void>('note_add_binding', { noteId, binding }),
    noteRemoveBinding: (noteId: string, binding: string) => call<void>('note_remove_binding', { noteId, binding }),
    getDir: () => call<{ current: string; is_custom: boolean }>('notes_get_dir'),
    setDir: (path: string | null) => call<{ current: string; is_custom: boolean }>('notes_set_dir', { path }),
    historyList: (noteId: string, filter?: HistoryFilter) =>
      call<NoteHistoryEntry[]>('note_history_list', { noteId, filter }),
    historyGet: (historyId: string) =>
      call<NoteHistoryEntry>('note_history_get', { historyId }),
    historyDiff: (noteId: string, fromId: string, toId: string) =>
      call<DiffResult>('note_history_diff', { noteId, fromId, toId }),
    historyRestore: (noteId: string, historyId: string) =>
      call<Note>('note_history_restore', { noteId, historyId }),
    historyMerge: (noteId: string, historyId: string) =>
      call<MergeResult>('note_history_merge', { noteId, historyId }),
    openWindow: (title: string) => call<void>('note_open_window', { title }),
    backlinks: (id: string) => call<NoteListItem[]>('note_backlinks', { id }),
    related: (id: string) => call<NoteListItem[]>('note_related', { id }),
    links: (id: string) => call<NoteLinks>('note_links', { id }),
    /** Live notes bound to or mentioning an entity binding, e.g. `ssh:{id}` */
    entityNotes: (binding: string) => call<NoteListItem[]>('note_entity_notes', { binding }),
    /** Names behind entity bindings; platform-neutral, public fields only */
    bindingSummaries: (bindings: string[]) => call<BindingSummary[]>('note_binding_summaries', { bindings }),
    entitySearch: (kind: string, query: string) => call<BindingSummary[]>('note_entity_search', { kind, query }),
    /** Placeholder name -> current value for a note's title and bindings */
    placeholderValues: (title: string, bindings: string[]) =>
      call<Record<string, string>>('note_placeholder_values', { title, bindings }),
    resolveLink: (target: string) => call<string | null>('note_resolve_link', { target }),
    syncInfo: (id: string) => call<NoteSyncInfo>('note_sync_info', { id }),
    smartViewList: () => call<NoteSmartView[]>('note_smart_view_list'),
    smartViewCreate: (input: SmartViewInput) => call<NoteSmartView>('note_smart_view_create', { input }),
    smartViewUpdate: (id: string, input: SmartViewInput) => call<NoteSmartView>('note_smart_view_update', { id, input }),
    smartViewDelete: (id: string) => call<void>('note_smart_view_delete', { id }),
    openQuickCapture: () => call<void>('open_quick_capture'),
    attachmentAdd: async (noteId: string, fileName: string, data: Uint8Array) => {
      // Raw body upload: metadata travels in headers
      const { invoke } = await import('@tauri-apps/api/core');
      return invoke<NoteAttachment>('note_attachment_add', data, {
        headers: { 'x-note-id': noteId, 'x-file-name': encodeURIComponent(fileName) },
      });
    },
    attachmentRead: async (noteId: string, name: string) =>
      base64ToBytes(await call<string>('note_attachment_read', { noteId, name })),
    /** Streaming copy of a native path or Android `content://` URI; no size limit from IPC. */
    attachmentAddFromPath: (noteId: string, srcPath: string) =>
      call<NoteAttachment>('note_attachment_add_from_path', { noteId, srcPath }),
    attachmentPolicyGet: () => call<NoteAttachmentPolicy>('notes_attachment_policy_get'),
    attachmentPolicySet: (policy: NoteAttachmentPolicy) =>
      call<NoteAttachmentPolicy>('notes_attachment_policy_set', { policy }),
    clipboardFilePaths: () => call<string[]>('clipboard_file_paths'),
    attachmentList: (noteId: string) => call<NoteAttachment[]>('note_attachment_list', { noteId }),
    /** Download a vault-only attachment; progress arrives via transfer events. */
    attachmentFetch: (noteId: string, name: string) =>
      call<NoteAttachment>('note_attachment_fetch', { noteId, name }),
    attachmentDelete: (noteId: string, name: string) => call<void>('note_attachment_delete', { noteId, name }),
    attachmentOpen: (noteId: string, name: string) => call<void>('note_attachment_open', { noteId, name }),
    attachmentSave: (noteId: string, name: string, dest: string) =>
      call<void>('note_attachment_save', { noteId, name, dest }),
    attachmentsGc: (del: boolean) => call<OrphanAttachment[]>('note_attachments_gc', { delete: del }),
    export: (ids: string[], dest: string, asZip: boolean, password?: string) =>
      call<{ count: number; path: string }>('note_export', { ids, dest, asZip, password: password ?? null }),
    import: (paths: string[], bindings: string[], password?: string) =>
      call<string[]>('note_import', { paths, bindings, password: password ?? null }),
  },
  sync: {
    ...core.sync,
    /** Stop a large attachment upload/download; it is retried on the next cycle. */
    attachmentCancel: (noteId: string, name: string) =>
      call<boolean>('sync_attachment_cancel', { noteId, name }),
    conflictGet: (noteId: string) => call<ConflictView>('sync_conflict_get', { noteId }),
    /** Fails with `conflict_changed` when the conflict moved since `conflictGet`. */
    conflictResolve: (noteId: string, token: string, content: string) =>
      call<SyncStatus>('sync_conflict_resolve', { noteId, token, content }),
  },
};

/** Save-as dialog, then copy the attachment to the chosen path. */
export async function downloadNoteAttachment(noteId: string, name: string): Promise<void> {
  const { save } = await import('@tauri-apps/plugin-dialog');
  const dest = await save({ defaultPath: name });
  if (!dest) return;
  await api.notes.attachmentSave(noteId, name, dest);
}

/** Notes-domain policy: when an attachment goes through chunked storage. */
export interface NoteAttachmentPolicy {
  large_files_enabled: boolean;
  /** Files at or above this size use chunked storage; smaller stay one blob. */
  threshold_mib: number;
  /** 0 = no product limit. */
  max_file_gib: number;
  /** Off: chunked attachments at or above `ask_above_mib` stay in the vault until requested. */
  download_on_sync: boolean;
  /** Missing attachments at or above this size prompt for download when the note opens. */
  ask_above_mib: number;
}

export const DEFAULT_ATTACHMENT_POLICY: NoteAttachmentPolicy = {
  large_files_enabled: true,
  threshold_mib: 16,
  max_file_gib: 10,
  download_on_sync: true,
  ask_above_mib: 16,
};

/** Progress of one attachment upload/download (`notes://attachment-transfer`). */
export interface AttachmentTransfer {
  note_id: string;
  name: string;
  direction: 'up' | 'down';
  phase: string;
  done: number;
  total: number | null;
  error: string | null;
  finished: boolean;
}

export const ATTACHMENT_TRANSFER_EVENT = 'notes://attachment-transfer';
