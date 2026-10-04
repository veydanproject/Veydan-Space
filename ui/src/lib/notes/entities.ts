// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * The notes module as owner in the catalog of entities (platform-spec 10.1):
 * the kind `note` (title only, as in Rust; references `kind:id` on a note
 * are added and removed through `link`), and what other modules show of the
 * notes' labels, folders and templates without reading the notes' store.
 */

import type { EntityKindDef, EntitySummary } from '$lib/core/module';
import { notesStore } from '$lib/notes/store/notes.svelte';
import { templateNotes } from '$lib/notes/filter';
import { isMobile } from '$lib/core/platform';

const loadAll = () => notesStore.ensureLoaded();

/** The labels alone (`note_tag_list`), once, for a reader that needs no notes (the phone's 2FA chips). */
let tagsLoading: Promise<void> | null = null;
const loadTags = (): Promise<void> => {
  if (notesStore.loaded) return Promise.resolve();
  tagsLoading ??= notesStore.refreshTags().catch((e) => {
    tagsLoading = null;
    throw e;
  });
  return tagsLoading;
};

/** A note of the list, or the one open in the editor (its bindings are the freshest). */
function noteBindings(id: string): string[] | undefined {
  if (notesStore.activeNote?.id === id) return notesStore.activeNote.bindings;
  return notesStore.list.find((n) => n.id === id)?.bindings;
}

export const noteEntities: EntityKindDef[] = [
  {
    kind: 'note',
    icon: 'file-text',
    label: 'ctx_kind_note',
    color: 'var(--accent)',
    ensureLoaded: loadAll,
    list: () =>
      notesStore.list
        .filter((n) => !n.deleted)
        .map((n) => ({ id: n.id, name: n.title, subtitle: '', status: null, color: 'var(--accent)' })),
    actions: [],
    // The phone opens a note on its own screen (and its conflict there); the
    // desktop selects it on the notes page, where the conflict shows itself.
    href: (id, view) => (isMobile ? `/notes/${id}${view === 'conflict' ? '?conflict=1' : ''}` : `/notes?open=${id}`),
    referring: (key) => notesStore.list.filter((n) => !n.deleted && n.bindings.includes(key)).map((n) => n.id),
    link: {
      add: async (id, reference) => {
        if (noteBindings(id)?.includes(reference)) return;
        await notesStore.addNoteBinding(id, reference);
      },
      remove: async (id, reference) => {
        // A note without the reference is left alone: a write would rewrite its manifest.
        if (!noteBindings(id)?.includes(reference)) return;
        await notesStore.removeNoteBinding(id, reference);
      },
    },
  },
  {
    kind: 'note_tag',
    icon: 'tag',
    label: 'ctx_kind_note_tag',
    color: 'var(--accent)',
    ensureLoaded: loadTags,
    list: () => notesStore.allTags.map((t) => ({ id: t.name, name: t.name, subtitle: '', status: null, color: t.color })),
    actions: [],
    create: async (name, color): Promise<EntitySummary> => {
      const tag = await notesStore.createTag(name, color);
      return { id: tag.name, name: tag.name, subtitle: '', status: null, color: tag.color };
    },
  },
  {
    kind: 'note_folder',
    icon: 'folder',
    label: 'ctx_kind_note_folder',
    color: 'var(--accent)',
    ensureLoaded: loadAll,
    list: () => notesStore.folders.map((f) => ({ id: f.id, name: f.name, subtitle: '', status: null, color: f.color })),
    actions: [],
  },
  {
    kind: 'note_template',
    icon: 'file-text',
    label: 'ctx_kind_note_template',
    color: 'var(--accent)',
    ensureLoaded: loadAll,
    list: () =>
      templateNotes(notesStore.list, notesStore.folders).map((n) => ({
        id: n.id, name: n.title, subtitle: '', status: null, color: 'var(--accent)',
      })),
    actions: [],
  },
];
