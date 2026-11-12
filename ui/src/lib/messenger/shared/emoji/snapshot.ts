// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The popular emoji of an open panel or menu are read once, on opening: a pick
// moves its emoji to the front in the store, and the row must not jump under
// the finger. Opened with nothing to show, the row follows the store until the
// first pick, then it is frozen too.

/** The row at opening: the popular, else the fallback; null while both are empty. */
export function takeSnapshot(popular: readonly string[], fallback: readonly string[] = []): string[] | null {
  if (popular.length) return [...popular];
  if (fallback.length) return [...fallback];
  return null;
}

/** What the row shows: the snapshot once there is one, else the live list. */
export function shown(snapshot: string[] | null, live: string[]): string[] {
  return snapshot ?? live;
}

/** The snapshot to keep at a pick: the one taken, else a copy of what was shown. */
export function freeze(snapshot: string[] | null, live: readonly string[]): string[] {
  return snapshot ?? [...live];
}
