// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * The list of notes and the note beside it on a wide window (core/ui/split):
 * what the list hears of the editor's writes, and what it closes at the right.
 */

/**
 * Reloads the list after a note's write, `ms` after the last one. Heard on
 * whatever address: the editor saves on leaving, so its last write lands once
 * the list's own address is back. The list's own actions reload it themselves
 * (`busy`).
 */
export function reloadOnWrites(
  subscribe: (cb: () => void) => () => void,
  load: () => void,
  busy: () => boolean,
  ms = 300,
): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const stop = subscribe(() => {
    if (busy()) return;
    clearTimeout(timer);
    timer = setTimeout(load, ms);
  });
  return () => {
    stop();
    clearTimeout(timer);
  };
}

/**
 * Empties the trash, then closes the note beside the list if it was in it:
 * the ids are of the whole trash, a search may hide the open one from the list.
 */
export async function emptyTrashClosing(
  notes: { list: (search: string, filter: { kind: 'trash' }) => Promise<{ id: string }[]>; emptyTrash: () => Promise<unknown> },
  close: (ids: string[]) => void,
): Promise<void> {
  const ids = (await notes.list('', { kind: 'trash' })).map((n) => n.id);
  await notes.emptyTrash();
  close(ids);
}
