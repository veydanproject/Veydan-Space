// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import type { NoteFilter, NoteFolder, NoteListItem, NoteSmartView } from '$lib/notes/types';
import { binding, isEntityKind } from '$lib/core/bindings';

export const TEMPLATES_FOLDER = 'Templates';

/** Live notes inside the `Templates` folder, usable as new-note templates */
export function templateNotes(list: NoteListItem[], folders: NoteFolder[]): NoteListItem[] {
  const ids = new Set(folders.filter((f) => f.name.toLowerCase() === TEMPLATES_FOLDER.toLowerCase()).map((f) => f.id));
  if (ids.size === 0) return [];
  return list.filter((n) => !n.deleted && n.folder_ids.some((id) => ids.has(id)));
}

/** Sidebar selection: what the user clicked */
export interface ActiveFilter {
  type: string;
  id?: string;
}

/** Map a sidebar selection to the backend filter */
export function toNoteFilter(f: ActiveFilter, smartViews: NoteSmartView[] = []): NoteFilter {
  // Entity kinds (workspace, profile, proxy, ssh, totp) filter by their binding
  if (isEntityKind(f.type) && f.id) return { archived: false, binding: binding(f.type, f.id) };
  switch (f.type) {
    case 'global':    return { archived: false, global_only: true };
    case 'domain':    return { archived: false, binding: `domain:${f.id}` };
    case 'tag':       return { archived: false, tag_name: f.id };
    case 'tag-group': return { archived: false, tag_prefix: f.id };
    case 'folder':    return { archived: false, folder_id: f.id };
    case 'pinned':    return { archived: false, pinned: true };
    case 'archived':  return { archived: true };
    case 'trash':     return { deleted: true };
    case 'smart': {
      const view = smartViews.find((v) => v.id === f.id);
      return view ? { archived: false, ...view.conditions } : { archived: false };
    }
    default:          return { archived: false };
  }
}

/** Bindings a new note inherits from the current selection */
export function contextBindings(f: ActiveFilter): string[] {
  return isEntityKind(f.type) && f.id ? [binding(f.type, f.id)] : [];
}
