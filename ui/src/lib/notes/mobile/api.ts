// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Mobile view of the notes' API: the same Rust commands as the desktop,
// reshaped for the phone screens (chips on cards, plain tag names, list
// filters by kind). The shell's part is $lib/core/mobile/api.

import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { api as shared, downloadNoteAttachment } from '$lib/notes/api';
import { api as core, onSyncChanged as onSyncChangedCore } from '$lib/core/mobile/api';
import { pickNativeFiles } from '$lib/notes/attachmentTransfer';
import { pickMediaFiles } from '$lib/notes/media/pick';
import type { MediaKind } from '$lib/notes/media/types';
import { capabilities } from '$lib/core/platform';
import { formatError } from '$lib/core/utils';
import { isEntityBinding, isEntityKind, parseBinding, type EntityKind } from '$lib/core/bindings';
import type {
  BindingSummary,
  ConflictView,
  MergeBlock,
  NavChild,
  Note as SharedNote,
  NoteAttachment,
  NoteFilter,
  NoteHistoryEntry,
  NoteLinks as SharedNoteLinks,
  NoteListItem as SharedNoteListItem,
  NoteNav,
  NoteSmartView,
  NoteSyncInfo,
  SmartViewInput,
} from '$lib/notes/types';
import type { SyncConfig, SyncProgress, SyncStatus } from '$lib/core/api';

export { onSyncProgress, onSyncStatus, syncProgressText } from '$lib/core/mobile/api';

export { formatError };
export type {
  BindingSummary,
  ConflictView,
  MergeBlock,
  NavChild,
  NoteAttachment,
  NoteFilter,
  NoteHistoryEntry,
  NoteNav,
  NoteSmartView,
  NoteSyncInfo,
  SmartViewInput,
  SyncConfig,
  SyncProgress,
  SyncStatus,
};

export interface NoteChip {
  kind: 'folder' | 'workspace' | 'profile' | 'domain' | 'tag' | string;
  id: string;
  label: string;
  color: string;
}

export interface NoteListItem {
  id: string;
  title: string;
  pinned: boolean;
  archived: boolean;
  created_at: string;
  updated_at: string;
  preview: string;
  tags: string[];
  /** `kind:value` bindings as stored on the note */
  bindings: string[];
  chips: NoteChip[];
  /** Shared item for views that need every field (table) */
  raw: SharedNoteListItem;
}

export interface Note extends Omit<NoteListItem, 'preview'> {
  content: string;
  contentHash: string | null;
}

export interface NoteUpdateInput {
  title?: string;
  content?: string;
  pinned?: boolean;
  archived?: boolean;
  tags?: string[];
  base_hash?: string | null;
}

export interface NoteTextMatch {
  id: string;
  title: string;
  updated_at: string;
  snippet: string;
  chips: NoteChip[];
}

export interface NoteSearchResult {
  notes: NoteListItem[];
  text_matches: NoteTextMatch[];
}

export interface NoteTag {
  id: string;
  name: string;
  color: string;
  count: number;
}

export interface NoteListFilter {
  kind: string;
  id?: string;
}

export interface NoteFolder {
  id: string;
  name: string;
  parent_id: string | null;
  color: string;
}

export interface NoteLinks {
  outgoing: NoteListItem[];
  backlinks: NoteListItem[];
  /** Link targets in the body that match no existing note */
  unresolved: string[];
}

// ── Shape adapters ───────────────────────────────────────────────────────────

const PROFILE_COLOR = '#8b7bff';
const DOMAIN_COLOR = '#888';
const ENTITY_COLOR: Record<EntityKind, string> = {
  workspace: '#34d399',
  profile: PROFILE_COLOR,
  proxy: '#f5c451',
  ssh: '#2dd4bf',
  totp: '#60a5fa',
  password: '#a78bfa',
};
const ENTITY_ICON: Record<EntityKind, string> = {
  workspace: 'layers',
  profile: 'globe',
  proxy: 'shield',
  ssh: 'terminal',
  totp: 'key',
  password: 'lock',
};

/** Names of entities seen so far (`note_binding_summaries`); filled before items are shaped. */
const summaryCache = new Map<string, BindingSummary>();

const needsSummary = (b: string) => isEntityBinding(b) && !summaryCache.has(b);

/** Fetch names for entity bindings not cached yet. */
async function warmSummaries(items: { bindings?: string[] }[]): Promise<void> {
  const missing = new Set<string>();
  for (const it of items) for (const b of it.bindings ?? []) if (needsSummary(b)) missing.add(b);
  if (missing.size === 0) return;
  for (const s of await shared.notes.bindingSummaries([...missing])) summaryCache.set(s.binding, s);
}

/** Cached summary, or null when the entity is unknown or not loaded yet. */
export function bindingSummary(binding: string): BindingSummary | null {
  return summaryCache.get(binding) ?? null;
}

/** Entities bound to a shared list item, for the table's context column. */
export function entitySummaryFor(item: SharedNoteListItem): { name: string; icon: string; color: string }[] {
  const out: { name: string; icon: string; color: string }[] = [];
  for (const b of item.bindings ?? []) {
    const p = parseBinding(b);
    const s = summaryCache.get(b);
    if (p && isEntityKind(p.kind) && s) out.push({ name: s.name, icon: ENTITY_ICON[p.kind], color: ENTITY_COLOR[p.kind] });
  }
  return out;
}

/** Chips in desktop order: folder, workspace, profile, other entities, domain, tag. */
function chipsFor(item: SharedNoteListItem, nav: NoteNav): NoteChip[] {
  const out: NoteChip[] = [];
  for (const fid of item.folder_ids ?? []) {
    const f = nav.folders.find((x) => x.id === fid);
    if (f) out.push({ kind: 'folder', id: f.id, label: f.name, color: f.color });
  }
  for (const b of item.bindings ?? []) {
    const p = parseBinding(b);
    if (!p) continue;
    if (p.kind === 'workspace') {
      const w = nav.all_workspaces.find((x) => x.id === p.value);
      if (w) out.push({ kind: 'workspace', id: p.value, label: w.name, color: w.color });
    } else if (p.kind === 'profile') {
      const pr = nav.all_profiles.find((x) => x.id === p.value);
      if (pr) out.push({ kind: 'profile', id: p.value, label: pr.name, color: PROFILE_COLOR });
    } else if (p.kind === 'domain') {
      if (p.value) out.push({ kind: 'domain', id: p.value, label: p.value, color: DOMAIN_COLOR });
    } else if (isEntityKind(p.kind)) {
      const s = summaryCache.get(b);
      if (s) out.push({ kind: p.kind, id: p.value, label: s.name, color: ENTITY_COLOR[p.kind] });
    }
  }
  for (const t of item.tags ?? []) out.push({ kind: 'tag', id: t.name, label: t.name, color: t.color });
  return out;
}

function toItem(d: SharedNoteListItem, nav: NoteNav): NoteListItem {
  return {
    id: d.id,
    title: d.title,
    pinned: d.pinned,
    archived: d.archived,
    created_at: d.created_at,
    updated_at: d.updated_at,
    preview: d.preview,
    tags: (d.tags ?? []).map((t) => t.name),
    bindings: d.bindings ?? [],
    chips: chipsFor(d, nav),
    raw: d,
  };
}

function toNote(d: SharedNote, nav: NoteNav): Note {
  const { preview: _preview, ...item } = toItem(d, nav);
  return { ...item, content: d.content ?? '', contentHash: d.content_hash };
}

/** Nav plus entity names for `items`, loaded together. */
async function shapeContext(items: { bindings?: string[] }[]): Promise<NoteNav> {
  const [nav] = await Promise.all([shared.notes.nav(), warmSummaries(items)]);
  return nav;
}

function toTag(t: { id: string; name: string; color: string; count?: number }): NoteTag {
  return { id: t.id, name: t.name, color: t.color, count: t.count ?? 0 };
}

/** Mobile list kinds mapped onto the shared `NoteFilter`. */
async function filterFor(q: NoteListFilter): Promise<NoteFilter> {
  const id = q.id ?? '';
  switch (q.kind) {
    case 'global': return { archived: false, global_only: true };
    case 'workspace': return { archived: false, binding: `workspace:${id}` };
    case 'profile': return { archived: false, binding: `profile:${id}` };
    case 'domain': return { archived: false, binding: `domain:${id}` };
    case 'tag': return { archived: false, tag_name: id };
    case 'folder': return { archived: false, folder_id: id };
    case 'pinned': return { archived: false, pinned: true };
    case 'archived': return { archived: true };
    case 'trash': return { deleted: true };
    case 'smart': {
      const view = (await shared.notes.smartViewList()).find((s) => s.id === id);
      const f: NoteFilter = { ...(view?.conditions ?? {}) };
      if (f.archived === undefined) f.archived = false;
      return f;
    }
    default: return { archived: false };
  }
}

const stripMarks = (s: string) => s.replace(/<\/?mark>/g, '');

// ── Events ───────────────────────────────────────────────────────────────────

/**
 * Runs `cb` when sync applied remote changes to one of `entities` ("note",
 * "note_attachment", "totp", ...). Note bodies and attachments are reported on
 * channels of their own.
 */
export async function onSyncChanged(entities: string[], cb: () => void): Promise<UnlistenFn> {
  const unlisteners: UnlistenFn[] = [await onSyncChangedCore(entities, cb)];
  if (entities.includes('note')) unlisteners.push(await listen('notes://external-change', cb));
  if (entities.includes('note_attachment')) unlisteners.push(await listen('notes://attachments-changed', cb));
  return () => unlisteners.forEach((f) => f());
}

/** Remote apply of one note body. Does not start a sync cycle. */
export function onNoteRemote(cb: (id: string) => void): Promise<UnlistenFn> {
  return listen<string>('notes://external-change', (e) => cb(e.payload));
}

// ── API ──────────────────────────────────────────────────────────────────────

export const api = {
  ...core,
  sync: {
    ...core.sync,
    conflictGet: (noteId: string) => shared.sync.conflictGet(noteId),
    conflictResolve: (noteId: string, token: string, content: string) =>
      shared.sync.conflictResolve(noteId, token, content),
    attachmentCancel: (noteId: string, name: string) => shared.sync.attachmentCancel(noteId, name),
  },
  notes: {
    nav: () => shared.notes.nav(),
    list: async (search = '', filter: NoteListFilter = { kind: 'all' }): Promise<NoteListItem[]> => {
      const items = await filterFor(filter).then((f) => shared.notes.list(f));
      const nav = await shapeContext(items);
      const q = search.trim().toLowerCase();
      return items
        .filter((n) => !q || n.title.toLowerCase().includes(q) || n.preview.toLowerCase().includes(q))
        .map((n) => toItem(n, nav));
    },
    get: async (id: string): Promise<Note> => {
      const n = await shared.notes.get(id);
      const nav = await shapeContext([n]);
      return toNote(n, nav);
    },
    create: async (title: string, content = '', tags: string[] = [], bindings: string[] = []): Promise<Note> => {
      const n = await shared.notes.create({ title, content, tag_names: tags, bindings });
      return toNote(n, await shapeContext([n]));
    },
    update: async (id: string, input: NoteUpdateInput): Promise<Note> => {
      if (input.tags) await shared.notes.setTags(id, input.tags);
      if (input.title !== undefined || input.content !== undefined || input.pinned !== undefined) {
        await shared.notes.update(id, {
          title: input.title,
          content: input.content,
          pinned: input.pinned,
          base_hash: input.base_hash,
        });
      }
      if (input.archived === true) await shared.notes.archive(id);
      else if (input.archived === false) await shared.notes.restore(id);
      const n = await shared.notes.get(id);
      const nav = await shapeContext([n]);
      return toNote(n, nav);
    },
    delete: (id: string) => shared.notes.delete(id),
    restore: async (id: string): Promise<Note> => {
      await shared.notes.restore(id);
      const n = await shared.notes.get(id);
      const nav = await shapeContext([n]);
      return toNote(n, nav);
    },
    purge: (id: string) => shared.notes.delete(id, true),
    emptyTrash: () => shared.notes.emptyTrash(),
    setFolder: (noteId: string, folderId: string | null) => shared.notes.noteSetFolder(noteId, folderId),
    addFolder: (noteId: string, folderId: string) => shared.notes.noteAddFolder(noteId, folderId),
    removeFolder: (noteId: string, folderId: string) => shared.notes.noteRemoveFolder(noteId, folderId),
    addBinding: (noteId: string, binding: string) => shared.notes.noteAddBinding(noteId, binding),
    removeBinding: (noteId: string, binding: string) => shared.notes.noteRemoveBinding(noteId, binding),
    /** Title hits first, body-only hits with an excerpt after. */
    search: async (query: string): Promise<NoteSearchResult> => {
      const q = query.trim().toLowerCase();
      const out: NoteSearchResult = { notes: [], text_matches: [] };
      if (!q) return out;
      const items = await shared.notes.search(query);
      const nav = await shapeContext(items);
      for (const d of items) {
        if (d.deleted) continue;
        if (d.title.toLowerCase().includes(q)) {
          out.notes.push(toItem(d, nav));
          continue;
        }
        out.text_matches.push({
          id: d.id,
          title: d.title,
          updated_at: d.updated_at,
          snippet: stripMarks(d.snippet ?? d.preview),
          chips: chipsFor(d, nav),
        });
      }
      return out;
    },
    tags: async (): Promise<NoteTag[]> => (await shared.notes.nav()).tags.map(toTag),
    tagCreate: async (name: string, color?: string) => toTag(await shared.notes.tagCreate(name, color)),
    tagUpdate: async (id: string, name?: string, color?: string) => toTag(await shared.notes.tagUpdate(id, name, color)),
    tagDelete: (id: string) => shared.notes.tagDelete(id),
    folderCreate: (name: string, parentId?: string, color?: string): Promise<NoteFolder> =>
      shared.notes.folderCreate(name, parentId, color),
    folderUpdate: (id: string, name?: string, color?: string): Promise<NoteFolder> =>
      shared.notes.folderUpdate(id, name, color),
    folderDelete: (id: string) => shared.notes.folderDelete(id),
    smartViewGet: async (id: string): Promise<NoteSmartView> => {
      const view = (await shared.notes.smartViewList()).find((s) => s.id === id);
      if (!view) throw new Error(`smart view ${id} not found`);
      return view;
    },
    smartViewCreate: (input: SmartViewInput) => shared.notes.smartViewCreate(input),
    smartViewUpdate: (id: string, input: SmartViewInput) => shared.notes.smartViewUpdate(id, input),
    smartViewDelete: (id: string) => shared.notes.smartViewDelete(id),
    historyList: (noteId: string) => shared.notes.historyList(noteId),
    historyGet: (id: string) => shared.notes.historyGet(id),
    historyRestore: async (noteId: string, historyId: string): Promise<Note> => {
      const n = await shared.notes.historyRestore(noteId, historyId);
      return toNote(n, await shapeContext([n]));
    },
    links: async (id: string): Promise<NoteLinks> => {
      const l: SharedNoteLinks = await shared.notes.links(id);
      const nav = await shapeContext([...l.outgoing, ...l.backlinks]);
      return {
        outgoing: l.outgoing.map((n) => toItem(n, nav)),
        backlinks: l.backlinks.map((n) => toItem(n, nav)),
        unresolved: l.unresolved,
      };
    },
    resolveLink: (target: string) => shared.notes.resolveLink(target),
    syncInfo: (id: string) => shared.notes.syncInfo(id),
    /** Live notes bound to or mentioning an entity, e.g. `ssh:{id}` */
    entityNotes: async (binding: string): Promise<NoteListItem[]> => {
      const items = await shared.notes.entityNotes(binding);
      const nav = await shapeContext(items);
      return items.map((n) => toItem(n, nav));
    },
    /** Names behind entity bindings; results are cached for chips */
    bindingSummaries: async (bindings: string[]): Promise<BindingSummary[]> => {
      await warmSummaries([{ bindings }]);
      return bindings.map((b) => summaryCache.get(b)).filter((s): s is BindingSummary => !!s);
    },
    entitySearch: (kind: string, query: string) => shared.notes.entitySearch(kind, query),
    placeholderValues: (title: string, bindings: string[]) => shared.notes.placeholderValues(title, bindings),
  },
  attachments: {
    list: (noteId: string) => shared.notes.attachmentList(noteId),
    fetch: (noteId: string, name: string) => shared.notes.attachmentFetch(noteId, name),
    /** System save dialog; on Android the `content://` target is written by Rust. */
    save: (noteId: string, name: string) => downloadNoteAttachment(noteId, name),
    /** Native picker; the `content://` URIs are streamed by Rust, nothing crosses IPC as bytes. */
    pick: async (noteId: string, imagesOnly: boolean): Promise<NoteAttachment[]> => {
      const out: NoteAttachment[] = [];
      for (const uri of await pickNativeFiles(imagesOnly)) out.push(await shared.notes.attachmentAddFromPath(noteId, uri));
      return out;
    },
    /** Native picker limited to audio or video files. */
    pickMedia: async (noteId: string, kind: MediaKind): Promise<NoteAttachment[]> => {
      const out: NoteAttachment[] = [];
      for (const uri of await pickMediaFiles(kind)) out.push(await shared.notes.attachmentAddFromPath(noteId, uri));
      return out;
    },
    /** Stores in-memory bytes; mobile goes through a temp file since raw IPC bodies are desktop-only. */
    addBlob: async (noteId: string, name: string, blob: Blob): Promise<NoteAttachment> => {
      const bytes = new Uint8Array(await blob.arrayBuffer());
      if (capabilities.hasRawIpc) return shared.notes.attachmentAdd(noteId, name, bytes);
      const { BaseDirectory, remove, writeFile } = await import('@tauri-apps/plugin-fs');
      const { appCacheDir, join } = await import('@tauri-apps/api/path');
      await writeFile(name, bytes, { baseDir: BaseDirectory.AppCache });
      try {
        return await shared.notes.attachmentAddFromPath(noteId, await join(await appCacheDir(), name));
      } finally {
        remove(name, { baseDir: BaseDirectory.AppCache }).catch(() => {});
      }
    },
    read: (noteId: string, name: string) => shared.notes.attachmentRead(noteId, name),
    delete: (noteId: string, name: string) => shared.notes.attachmentDelete(noteId, name),
  },
};
