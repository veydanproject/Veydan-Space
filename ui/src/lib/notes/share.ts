// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What another app shared into Veydan (`core/shares`) as a new note: the
// text becomes the note, its first line the title, the files its attachments.
// The notes take a share themselves where no messenger is (Notes alone) and
// offer «Новая заметка» in the messenger's chooser otherwise.

import { get } from 'svelte/store';
import { goto } from '$app/navigation';
import { t } from '$lib/core/i18n';
import { toast } from '$lib/core/ui/toast.svelte';
import { sharesApi, type IncomingShare } from '$lib/core/shares/api';
import { api } from './api';

/** The longest title made of a shared text (as the quick note's). */
const TITLE_MAX = 120;

/**
 * The title and the body of the note a share makes: the subject (a page's
 * title) above the whole text; else the first line of the text, the rest
 * under it; else the name of the first file; else `untitled`.
 */
export function noteOf(share: Pick<IncomingShare, 'text' | 'subject' | 'files'>, untitled: string): { title: string; content: string } {
  const text = (share.text ?? '').replace(/\r/g, '').trim();
  const subject = share.subject?.trim();
  if (subject) return { title: subject.slice(0, TITLE_MAX), content: text };
  if (text) {
    const lines = text.split('\n');
    const at = lines.findIndex((l) => l.trim());
    const title = lines[at].replace(/^#+\s*/, '').trim().slice(0, TITLE_MAX);
    return { title, content: lines.slice(at + 1).join('\n').trim() };
  }
  const first = share.files[0]?.name.trim();
  return { title: first ? first.slice(0, TITLE_MAX) : untitled, content: '' };
}

/**
 * The note made and opened; the files attached by their paths (the shell
 * vouches for them), one the attachment policy refuses is named. The copies
 * go once every file was tried.
 */
export async function createNoteFrom(share: IncomingShare): Promise<void> {
  const tr = get(t);
  const { title, content } = noteOf(share, tr('note_share_untitled'));
  const refused = [...share.refused];
  let noteId: string;
  try {
    const note = await api.notes.create({ title, content, bindings: [] });
    noteId = note.id;
    for (const f of share.files) {
      try {
        await api.notes.attachmentAddFromPath(note.id, f.path);
      } catch {
        refused.push(f.name);
      }
    }
  } finally {
    void sharesApi.forget(share.id);
  }
  if (refused.length) toast.error(tr('note_share_refused', { names: refused.join(', ') }), { action: false });
  else toast.success(tr('note_share_saved'));
  await goto(`/notes/${noteId}`);
}
