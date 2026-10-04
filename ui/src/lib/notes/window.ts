// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { windowLabel } from '$lib/core/api';
import type { Key } from '$lib/core/module';

/** The notes' own windows (`crates/notes`: the popout and quick capture) and their titles. */
export const NOTES_WINDOWS: { label: string; title: Key }[] = [
  { label: 'notes', title: 'nav_notes' },
  { label: 'quick-capture', title: 'quick_capture_title' },
];

/** True inside a notes window (popout or quick capture): theme + chrome only. */
export function isNotesWindow(): boolean {
  const label = windowLabel();
  return NOTES_WINDOWS.some((w) => w.label === label);
}
