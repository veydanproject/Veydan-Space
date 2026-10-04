// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The texts of the failures of the network panel. The runtime answers with
// stable codes inside its message; anything else is shown as it came.

export type NetErrorKey = 'msg_bridge_error_invalid' | 'msg_bridge_error_link_type';

/** Key of the text for a failure: `msg_bridge_error_<code>`, or null for the text itself. */
export function netErrorKey(error: string): NetErrorKey | null {
  // A well-formed link of another kind, such as the old `veydan://bridge/…`.
  if (error.includes('net_bridge_link_type')) return 'msg_bridge_error_link_type';
  return error.includes('net_bridge_invalid') ? 'msg_bridge_error_invalid' : null;
}

/**
 * The text every screen shows for a failure: the translated one for a known
 * code, the runtime's own wording otherwise, '' for no failure.
 */
export function netErrorText(error: string, translate: (key: NetErrorKey) => string): string {
  if (!error) return '';
  const key = netErrorKey(error);
  return key ? translate(key) : error;
}
