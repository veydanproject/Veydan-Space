// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Where the restore by 12 words and the device linking say that the archive of
// chats starts coming back (messenger-archive-plan.md section 5, step B): the
// messenger holds its first screen for 20 s at most. A key of the session and
// an event of the window in the core, so the cloud pages and the messenger know
// nothing of each other (scripts/boundaries.sh) and
// a page reloaded after a linking still finds the hold.

/** The session key: the time (ms) the hold began. */
export const ARCHIVE_GATE_KEY = 'veydan.archive.gate';

/** The window event the hold is announced with, for a page that stays. */
export const ARCHIVE_GATE_EVENT = 'veydan:archive-gate';

/** An account was restored or linked on this device: hold the first screen while the archive comes back. */
export function holdArchiveGate(): void {
  try {
    sessionStorage.setItem(ARCHIVE_GATE_KEY, String(Date.now()));
  } catch { /* no storage: the hold lasts for this page only */ }
  if (typeof window !== 'undefined') window.dispatchEvent(new Event(ARCHIVE_GATE_EVENT));
}
