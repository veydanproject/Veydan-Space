// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Note editor helpers shared by the page and its sheets.

export type EditorMode = 'rich' | 'md';

/** Which list a note row lives in; decides its swipe buttons. */
export type RowMode = 'normal' | 'archived' | 'trash';
export type RowAction = 'pin' | 'archive' | 'unarchive' | 'delete' | 'restore' | 'purge';

const MODE_KEY = 'notes.editorMode';

export function loadEditorMode(): EditorMode {
  return localStorage.getItem(MODE_KEY) === 'md' ? 'md' : 'rich';
}

export function saveEditorMode(mode: EditorMode) {
  localStorage.setItem(MODE_KEY, mode);
}

/** Short date + time in the user's locale. */
export function fmtDateTime(iso: string, locale: string): string {
  return new Date(iso).toLocaleString(locale, { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });
}

/** Full date + time for the note meta block. */
export function fmtDateTimeLong(iso: string, locale: string): string {
  return new Date(iso).toLocaleString(locale, { day: 'numeric', month: 'short', year: 'numeric', hour: '2-digit', minute: '2-digit' });
}

/** List date: time for today, otherwise "20 Sep". */
export function fmtListDate(iso: string, locale: string): string {
  const d = new Date(iso);
  const sameDay = d.toDateString() === new Date().toDateString();
  return sameDay
    ? d.toLocaleTimeString(locale, { hour: '2-digit', minute: '2-digit' })
    : d.toLocaleDateString(locale, { day: 'numeric', month: 'short' });
}

/** File size for attachment cards. */
export function fmtSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** Card colour: folder first, then workspace, else the accent. */
export function noteColor(chips: { kind: string; color: string }[]): string {
  const c = chips.find((x) => x.kind === 'folder') ?? chips.find((x) => x.kind === 'workspace');
  return c?.color || 'var(--accent)';
}
