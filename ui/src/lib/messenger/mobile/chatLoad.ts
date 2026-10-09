// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// When the phone chat page loads its chat. Pure, so the rule is tested
// without the page.

export interface ChatPageState {
  /** The chat in the address. */
  id: string;
  /** A load is under way. */
  loading: boolean;
  /** The chat the page loaded last. */
  loadedFor: string | null;
  /** The resets of the store when the page loaded it. */
  loadedEpoch: number;
  /** The resets of the store now. */
  resets: number;
  /** A chat is open in the store. */
  active: boolean;
  /** The chat found not on this device. */
  missingId: string | null;
  /** The chat is in the list of the store. */
  known: boolean;
}

/**
 * A chat new to the page is loaded. So is this one again when the store was
 * emptied under the page (the app came back locked), or when it turns up
 * after it was missing. A chat closed on purpose (deleted, forgotten,
 * archived) is not opened again: the page is on its way out.
 */
export function shouldLoad(s: ChatPageState): boolean {
  if (!s.id || s.loading) return false;
  const again = !s.active && (s.missingId === s.id ? s.known : s.resets !== s.loadedEpoch);
  return s.loadedFor !== s.id || again;
}
