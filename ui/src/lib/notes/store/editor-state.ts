// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the desktop editor takes from the store, as plain functions: which
// note the editor may type into, and what a save answer changes in the open note.

import type { Note } from '$lib/notes/types';

/**
 * The editor shows the note that is open: until the note picked last has loaded,
 * the one on screen is the previous note, and what is typed into it belongs to neither.
 */
export function editorReady(activeNoteId: string | null, activeNote: Note | null): boolean {
  return activeNoteId !== null && activeNote !== null && activeNote.id === activeNoteId;
}

/**
 * The open note after a save answered: the fields the save changed come from the
 * answer, the body stays the one the editor was given (what was typed since the
 * save was sent is newer than the answer), and a title typed since stays too.
 */
export function afterSave(open: Note, saved: Note, pendingTitle: string | null): Note {
  return { ...saved, content: open.content, title: pendingTitle ?? saved.title };
}

/** The buffer is saved only into the note it was typed in. */
export function bufferFor(pendingFor: string | null, noteId: string): boolean {
  return pendingFor !== null && pendingFor === noteId;
}
