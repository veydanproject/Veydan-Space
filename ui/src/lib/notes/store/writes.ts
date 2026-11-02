// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Writes of a note still on their way. A note left while its save runs and opened again reads
// its file only after them: read before, the file is the one before the save, and the editor
// shows the text typed last as lost (an empty new note shows its placeholder).

const writes = new Map<string, Promise<void>>();

/** `p` writes note `id`; a read of the note through `writesSettled` waits for it. */
export function trackWrite<T>(id: string, p: Promise<T>): Promise<T> {
  const done = p.then(() => {}, () => {});
  const before = writes.get(id);
  const all = before ? before.then(() => done) : done;
  writes.set(id, all);
  void all.then(() => {
    if (writes.get(id) === all) writes.delete(id);
  });
  return p;
}

/** Resolves once the writes of note `id` begun so far have ended, whatever their end. */
export async function writesSettled(id: string): Promise<void> {
  await writes.get(id);
}
