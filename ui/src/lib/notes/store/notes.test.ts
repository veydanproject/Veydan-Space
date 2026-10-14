// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The desktop editor keeps what is typed: a save answer, a title, a pin leave the body in the
// editor; only a note opened or a body put in from outside (contentRev) replace it; and a body
// typed in one note is never written into another.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Note, NoteUpdateInput } from '$lib/notes/types';
import { afterSave, bufferFor, editorReady } from './editor-state';

type Gate<T> = { promise: Promise<T>; resolve: (v: T) => void; reject: (e: unknown) => void };
function gate<T>(): Gate<T> {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

const notes = vi.hoisted(() => ({
  get: vi.fn(),
  update: vi.fn(),
  list: vi.fn(async () => []),
  search: vi.fn(async () => []),
  draftSave: vi.fn(async () => {}),
  draftGet: vi.fn(async (): Promise<string | null> => null),
}));
vi.mock('$lib/notes/api', () => ({ api: { notes } }));

const { notesStore } = await import('./notes.svelte');

function note(id: string, content: string, hash = `h:${content}`, extra: Partial<Note> = {}): Note {
  return {
    id,
    title: `Title ${id}`,
    bindings: [],
    tags: [],
    folder_ids: [],
    pinned: false,
    archived: false,
    deleted: false,
    doc_status: 'draft',
    created_at: '2026-10-01T00:00:00Z',
    updated_at: '2026-10-01T00:00:00Z',
    has_draft: false,
    preview: '',
    file_path: `${id}.md`,
    base_dir: '/notes',
    content_hash: hash,
    content,
    ...extra,
  } as Note;
}

/** What the backend would answer to an update of `prev` with `input`. */
function saved(prev: Note, input: NoteUpdateInput): Note {
  const content = input.content ?? prev.content ?? '';
  return { ...prev, content, content_hash: `h:${content}`, title: input.title ?? prev.title, updated_at: '2026-10-02T00:00:00Z' };
}

const updateCalls = () => notes.update.mock.calls.map(([id, input]) => [id, input] as [string, NoteUpdateInput]);

async function open(n: Note) {
  notes.get.mockResolvedValueOnce(n);
  await notesStore.openNote(n.id);
}

const store = notesStore as unknown as {
  _clearEditorState(): void;
  _handleExternal(id: string): Promise<void>;
  stopWatcher(): void;
};

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  store._clearEditorState();
  store.stopWatcher();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('editor state helpers', () => {
  it('takes input only into the note that is open', () => {
    const a = note('a', 'A');
    expect(editorReady('a', a)).toBe(true);
    expect(editorReady('b', a)).toBe(false);
    expect(editorReady('a', null)).toBe(false);
    expect(editorReady(null, a)).toBe(false);
  });

  it('keeps the body of the open note and a title typed since', () => {
    const open = note('a', 'Hello');
    const answer = note('a', 'Hello w', 'h1', { title: 'Old', pinned: true });
    const next = afterSave(open, answer, 'Newer');
    expect(next.content).toBe('Hello');
    expect(next.title).toBe('Newer');
    expect(next.pinned).toBe(true);
    expect(next.content_hash).toBe('h1');
    expect(afterSave(open, answer, null).title).toBe('Old');
  });

  it('saves a buffer only into the note it was typed in', () => {
    expect(bufferFor('a', 'a')).toBe(true);
    expect(bufferFor('a', 'b')).toBe(false);
    expect(bufferFor(null, 'a')).toBe(false);
  });
});

describe('a save answer', () => {
  it('does not take back what was typed while it ran', async () => {
    const a = note('a', 'Hello', 'h0');
    await open(a);
    const rev = notesStore.contentRev;

    const first = gate<Note>();
    notes.update.mockReturnValueOnce(first.promise);
    notesStore.onContentChange('Hello w');
    await vi.advanceTimersByTimeAsync(3000);
    expect(updateCalls()).toEqual([['a', { content: 'Hello w', base_hash: 'h0' }]]);

    // Typed while the save is on its way.
    notesStore.onContentChange('Hello world');
    first.resolve(saved(a, { content: 'Hello w' }));
    await vi.advanceTimersByTimeAsync(0);

    expect(notesStore.contentRev).toBe(rev);
    expect(notesStore.activeNote?.content).toBe('Hello');
    expect(notesStore.activeNote?.content_hash).toBe('h:Hello w');
    expect(notesStore.saveStatus).toBe('unsaved');

    notes.update.mockImplementationOnce(async (_id: string, input: NoteUpdateInput) => saved(a, input));
    await vi.advanceTimersByTimeAsync(3000);
    expect(updateCalls()[1]).toEqual(['a', { content: 'Hello world', base_hash: 'h:Hello w' }]);
    expect(notesStore.saveStatus).toBe('saved');
    expect(notesStore.contentRev).toBe(rev);
  });

  it('of a title leaves the body that is typed', async () => {
    const a = note('a', 'Hello', 'h0');
    await open(a);
    const rev = notesStore.contentRev;

    notesStore.onContentChange('Hello world');
    notesStore.onTitleChange('Title!');
    expect(notesStore.activeNote?.title).toBe('Title!');
    expect(notesStore.contentRev).toBe(rev);

    notes.update.mockImplementationOnce(async (_id: string, input: NoteUpdateInput) => saved(a, input));
    await vi.advanceTimersByTimeAsync(3000);
    expect(updateCalls()).toEqual([['a', { content: 'Hello world', base_hash: 'h0', title: 'Title!' }]]);
    expect(notesStore.contentRev).toBe(rev);
    expect(notesStore.activeNote?.title).toBe('Title!');
  });

  it('of a pin keeps the body and a title being typed', async () => {
    const a = note('a', 'Hello', 'h0');
    notesStore.list = [{ ...a }];
    await open(a);
    const rev = notesStore.contentRev;
    notesStore.onTitleChange('Typed');
    notes.update.mockResolvedValueOnce({ ...a, pinned: true });
    await notesStore.togglePin('a');
    expect(notesStore.activeNote?.pinned).toBe(true);
    expect(notesStore.activeNote?.title).toBe('Typed');
    expect(notesStore.contentRev).toBe(rev);
  });
});

describe('switching notes', () => {
  it('never writes the text of the previous note into the next one', async () => {
    const a = note('a', 'Alpha', 'hA');
    const b = note('b', 'Beta', 'hB');
    await open(a);
    notesStore.onContentChange('Alpha typed');

    const loadB = gate<Note>();
    notes.get.mockReturnValueOnce(loadB.promise);
    notes.update.mockImplementation(async (id: string, input: NoteUpdateInput) => saved(id === 'a' ? a : b, input));
    const opening = notesStore.openNote('b');

    // The previous note goes out on its own hash.
    expect(updateCalls()).toEqual([['a', { content: 'Alpha typed', base_hash: 'hA' }]]);
    expect(notesStore.activeNoteId).toBe('b');
    expect(notesStore.activeNote?.id).toBe('a');
    expect(notesStore.noteLoading).toBe(true);

    // The editor still shows A; what is typed there belongs to neither note.
    notesStore.onContentChange('Alpha typed more');
    notesStore.onTitleChange('Alpha title');
    expect(notesStore.saveStatus).toBe('saved');

    loadB.resolve(b);
    await opening;
    expect(notesStore.noteLoading).toBe(false);
    expect(notesStore.activeNote?.content).toBe('Beta');
    await vi.advanceTimersByTimeAsync(10_000);

    expect(updateCalls()).toEqual([['a', { content: 'Alpha typed', base_hash: 'hA' }]]);
    expect(notes.draftSave).not.toHaveBeenCalled();
    notes.update.mockReset();
  });

  it('takes no input into an old copy of a note opened again while it loads (A, B, A)', async () => {
    const a = note('a', 'Alpha', 'hA');
    await open(a);
    const loadB = gate<Note>();
    notes.get.mockReturnValueOnce(loadB.promise);
    const toB = notesStore.openNote('b');
    const loadA = gate<Note>();
    notes.get.mockReturnValueOnce(loadA.promise);
    const toA = notesStore.openNote('a');

    // The copy of A on screen is the one from before; A is loading again.
    expect(notesStore.activeNote?.id).toBe('a');
    expect(notesStore.noteLoading).toBe(true);
    notesStore.onContentChange('Alpha stale');
    expect(notesStore.saveStatus).toBe('saved');

    loadB.resolve(note('b', 'Beta'));
    loadA.resolve(note('a', 'Alpha 2', 'hA2'));
    await Promise.all([toB, toA]);
    expect(notesStore.noteLoading).toBe(false);
    expect(notesStore.activeNote?.content).toBe('Alpha 2');
    notes.update.mockImplementationOnce(async (_id: string, input: NoteUpdateInput) => saved(a, input));
    notesStore.onContentChange('Alpha 2!');
    await vi.advanceTimersByTimeAsync(3000);
    expect(updateCalls()).toEqual([['a', { content: 'Alpha 2!', base_hash: 'hA2' }]]);
  });

  it('keeps the previous body as a draft when its save is refused', async () => {
    const a = note('a', 'Alpha', 'hA');
    await open(a);
    notesStore.onContentChange('Alpha typed');
    notes.update.mockRejectedValueOnce({ code: 'conflict_changed', message: 'note a' });
    await open(note('b', 'Beta'));
    await vi.advanceTimersByTimeAsync(0);
    expect(notes.draftSave).toHaveBeenCalledWith('a', 'Alpha typed');
  });

  it('shows no note when the next one fails to load, and loads it again on the next click', async () => {
    await open(note('a', 'Alpha'));
    notes.get.mockRejectedValueOnce(new Error('gone'));
    await notesStore.openNote('b');
    expect(notesStore.activeNoteId).toBe('b');
    expect(notesStore.activeNote).toBe(null);

    await open(note('b', 'Beta'));
    expect(notesStore.activeNote?.content).toBe('Beta');
  });
});

describe('contentRev', () => {
  it('grows when the store puts a body into the editor', async () => {
    const a = note('a', 'Alpha', 'hA');
    let rev = notesStore.contentRev;
    await open(a);
    expect(notesStore.contentRev).toBe(++rev);

    // A change from outside, nothing pending: taken in.
    notes.get.mockResolvedValueOnce(note('a', 'Remote', 'hR'));
    await store._handleExternal('a');
    expect(notesStore.activeNote?.content).toBe('Remote');
    expect(notesStore.contentRev).toBe(++rev);

    // A change from outside with a buffer: the banner, then "take theirs".
    notesStore.onContentChange('Mine');
    notes.get.mockResolvedValueOnce(note('a', 'Remote 2', 'hR2'));
    await store._handleExternal('a');
    expect(notesStore.externalChange).toBe(true);
    expect(notesStore.contentRev).toBe(rev);
    notes.get.mockResolvedValueOnce(note('a', 'Remote 2', 'hR2'));
    await notesStore.acceptExternalChange();
    expect(notesStore.contentRev).toBe(++rev);

    // A recovered draft.
    notes.draftGet.mockResolvedValueOnce('From draft');
    await notesStore.recoverDraft('a');
    expect(notesStore.activeNote?.content).toBe('From draft');
    expect(notesStore.contentRev).toBe(++rev);

    // A restored version: the buffer goes with it and the next save is on its hash.
    notesStore.takeRestored(note('a', 'Restored', 'hOld'));
    expect(notesStore.contentRev).toBe(++rev);
    expect(notesStore.saveStatus).toBe('saved');
    await vi.advanceTimersByTimeAsync(10_000);
    expect(notes.update).not.toHaveBeenCalled();
    notes.update.mockImplementationOnce(async (_id: string, input: NoteUpdateInput) => saved(a, input));
    notesStore.onContentChange('Restored!');
    await vi.advanceTimersByTimeAsync(3000);
    expect(updateCalls()).toEqual([['a', { content: 'Restored!', base_hash: 'hOld' }]]);
  });

  it('stays when "keep mine" writes the buffer the editor already shows', async () => {
    await open(note('a', 'Alpha', 'hA'));
    notesStore.onContentChange('Mine');
    notes.get.mockResolvedValueOnce(note('a', 'Remote', 'hR'));
    await store._handleExternal('a');
    const rev = notesStore.contentRev;

    notes.get.mockResolvedValueOnce(note('a', 'Remote', 'hR'));
    notes.update.mockImplementationOnce(async (_id: string, input: NoteUpdateInput) => saved(note('a', 'Remote', 'hR'), input));
    await notesStore.discardExternalChange();
    expect(updateCalls()).toEqual([['a', { content: 'Mine', base_hash: 'hR' }]]);
    expect(notesStore.externalChange).toBe(false);
    expect(notesStore.contentRev).toBe(rev);
  });
});

describe('external changes of other notes', () => {
  it('reload the lists once for a burst of events', async () => {
    notes.list.mockClear();
    await Promise.all(['x', 'y', 'z', 'x'].map((id) => store._handleExternal(id)));
    expect(notes.list).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(149);
    expect(notes.list).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(2);
    // one refresh: the full list and the view
    expect(notes.list).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(1000);
    expect(notes.list).toHaveBeenCalledTimes(2);
  });
});
