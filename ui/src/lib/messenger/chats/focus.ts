// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * Which chat I am reading on this device, as the runtime is told it
 * (`messenger_focus`), so that my other devices show nothing for it
 * (ROO-1275). The decisions only; `focusLease` in `focus.svelte.ts` wires
 * them to the page.
 */

/**
 * The runtime holds a word for five minutes (`FOCUS_IDLE_SECS`); a touch
 * renews it, no more often than this.
 */
export const FOCUS_RENEW_MS = 30_000;

export interface FocusSight {
  /** The chats are in sight: the page is visible and not behind the lock (`inSight`). */
  seen: boolean;
  /** The window has the focus of the system. */
  focused: boolean;
  /** The page of the messenger is the one shown. */
  inMessenger: boolean;
  /** The chat open in it, if any. */
  active: string | null;
}

/** The chat being read now, or none. */
export function focusOf(s: FocusSight): string | null {
  return s.seen && s.focused && s.inMessenger ? s.active : null;
}

/**
 * Whether to tell the runtime `chat` now, `said` having been told at
 * `saidAt`: a change at once; the same chat again after half a minute,
 * to renew its word; no chat never twice.
 */
export function focusToSay(chat: string | null, said: string | null, saidAt: number, now: number): boolean {
  if (chat !== said) return true;
  return chat !== null && now - saidAt >= FOCUS_RENEW_MS;
}
