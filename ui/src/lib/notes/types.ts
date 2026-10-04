// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Types of the notes module's commands.

export type NoteFormat = 'md' | 'txt' | 'py' | string;

export type NoteDocStatus = 'active' | 'orphan' | 'missing';

export type SaveStatus = 'saved' | 'saving' | 'unsaved' | 'failed' | 'external';

export interface NoteTagInfo {
  id: string;
  name: string;
  color: string;
}

export interface NoteTag {
  id: string;
  name: string;
  color: string;
  created_at: string;
  updated_at: string;
}

export interface NoteFolder {
  id: string;
  name: string;
  parent_id: string | null;
  color: string;
  created_at: string;
  updated_at: string;
}

export interface NoteListItem {
  id: string;
  title: string;
  format: NoteFormat;
  /** Context bindings, e.g. ["workspace:id", "profile:id"] */
  bindings: string[];
  tags: NoteTagInfo[];
  folder_ids: string[];
  pinned: boolean;
  archived: boolean;
  deleted: boolean;
  doc_status: NoteDocStatus;
  created_at: string;
  updated_at: string;
  has_draft: boolean;
  preview: string;
  snippet?: string;
}

export interface Note extends NoteListItem {
  file_path: string;
  /** Absolute dir of the note file; relative attachment links resolve against it */
  base_dir: string;
  content_hash: string | null;
  content: string | null;
}

export interface NoteAttachment {
  name: string;
  /** `attachments/{note_id}/{name}` — relative to the note file */
  rel_path: string;
  size: number;
  is_image: boolean;
  /** False for a chunked file that is still only in the vault. */
  present: boolean;
}

export interface OrphanAttachment {
  note_id: string;
  name: string;
  size: number;
}

export interface NoteCreateInput {
  title: string;
  format?: NoteFormat;
  /** Context bindings, e.g. ["workspace:id", "profile:id"] */
  bindings?: string[];
  tag_names?: string[];
  content?: string;
  /** Note from the Templates folder whose rendered body becomes the content */
  template_id?: string;
}

export interface NoteUpdateInput {
  title?: string;
  content?: string;
  pinned?: boolean;
  /** Hash of the body this editor loaded. Sent with content so a stale buffer is rejected. */
  base_hash?: string | null;
}

/** List filter; also the persisted condition set of a smart view */
export interface NoteFilter {
  /** Filter notes that contain this binding, e.g. "workspace:id" or "profile:id" */
  binding?: string;
  tag_name?: string;
  /** Folder and all of its descendants */
  folder_id?: string;
  pinned?: boolean;
  /** undefined = any, true/false = only that state */
  archived?: boolean;
  /** true = trash only; otherwise live notes */
  deleted?: boolean;
  /** Tag `name` or any `name/...` sub-tag */
  tag_prefix?: string;
  /** Notes without workspace/profile bindings */
  global_only?: boolean;
  bindings_any?: string[];
  tags_any?: string[];
  tags_all?: string[];
  updated_within_days?: number;
  has_attachments?: boolean;
  has_open_tasks?: boolean;
}

export interface NoteSmartView {
  id: string;
  name: string;
  color: string;
  conditions: NoteFilter;
  sort_order: number;
}

export interface SmartViewInput {
  name: string;
  color?: string;
  conditions: NoteFilter;
}

export interface NavChild {
  id: string;
  name: string;
  color: string;
  count: number;
  parent_id?: string | null;
  profiles?: NavChild[];
}

export interface NavTag extends NoteTagInfo {
  count: number;
}

export interface NoteNav {
  counts: { all: number; global: number; pinned: number; archived: number; trash: number };
  /** Workspaces with notes; profiles nested under their workspace. */
  workspaces: NavChild[];
  folders: NavChild[];
  tags: NavTag[];
  smart: NavChild[];
  sites: NavChild[];
  /** Full catalogs for pickers, no counts. */
  all_workspaces: NavChild[];
  all_profiles: NavChild[];
}

export interface NoteLinks {
  outgoing: NoteListItem[];
  backlinks: NoteListItem[];
  /** Link targets in the body that match no existing note */
  unresolved: string[];
}

/** Public description of the entity behind a `kind:id` binding */
export interface BindingSummary {
  binding: string;
  kind: string;
  name: string;
  subtitle: string;
}

export interface NoteSyncInfo {
  /** The vault knows this note. */
  tracked: boolean;
  /** Local edits not yet pushed. */
  pending: boolean;
}

export type VersionType = 'save' | 'autosave' | 'restore' | 'merge' | 'import' | 'sync' | 'conflict';

export interface NoteHistoryEntry {
  id: string;
  note_id: string;
  parent_id: string | null;
  revision: number;
  version_type: VersionType;
  title: string;
  content?: string;
  content_hash: string;
  author: string | null;
  device: string | null;
  created_at: string;
}

export interface MergeBlock {
  kind: 'normal' | 'conflict';
  /** Normal block text (keeps its trailing newline) */
  text: string;
  ours: string;
  theirs: string;
}

export interface MergeResult {
  content: string;
  has_conflicts: boolean;
  blocks: MergeBlock[];
}

/** Sync conflict snapshot; `token` must be sent back with the resolution. */
export interface ConflictView {
  token: string;
  merge: MergeResult;
  /** This device's sync name. Empty when the user has not set one. */
  local_device: string;
  /** Other device's sync name. Empty when the name is unknown. */
  remote_device: string;
}

export interface DiffLine {
  kind: 'context' | 'added' | 'removed';
  content: string;
}

export interface DiffResult {
  lines: DiffLine[];
}

export interface HistoryFilter {
  version_type?: VersionType;
  date_from?: string;
  date_to?: string;
}
