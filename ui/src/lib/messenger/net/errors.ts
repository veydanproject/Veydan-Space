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

// ─── The call nodes ───────────────────────────────────────────────────────

export type CallNodeErrorKey =
  | 'msg_calls_node_invalid'
  | 'msg_call_nodes_err_link_type'
  | 'msg_call_nodes_err_no_invites'
  | 'msg_call_nodes_err_bad_invite'
  | 'msg_call_nodes_err_rate_limited'
  | 'msg_call_nodes_err_other'
  | 'msg_call_nodes_err_unreachable'
  | 'msg_call_nodes_err_unknown'
  | 'msg_call_nodes_err_locked';

/** The runtime's codes (`call_node_*`) and the key of each one's text. */
const CALL_NODE_CODES: [string, CallNodeErrorKey][] = [
  ['call_node_invalid', 'msg_calls_node_invalid'],
  ['call_node_link_type', 'msg_call_nodes_err_link_type'],
  ['call_node_no_invites', 'msg_call_nodes_err_no_invites'],
  ['call_node_bad_invite', 'msg_call_nodes_err_bad_invite'],
  ['call_node_rate_limited', 'msg_call_nodes_err_rate_limited'],
  ['call_node_other', 'msg_call_nodes_err_other'],
  ['call_node_unreachable', 'msg_call_nodes_err_unreachable'],
  ['call_node_unknown', 'msg_call_nodes_err_unknown'],
  ['call_node_locked', 'msg_call_nodes_err_locked'],
];

/** Key of the text for a failure of the call nodes, or null for the text itself. */
export function callNodeErrorKey(error: string): CallNodeErrorKey | null {
  // `call_node_unreachable: <what the way said>`, maybe after a word of the host.
  const code = /call_node_[a-z_]+/.exec(error)?.[0];
  return CALL_NODE_CODES.find(([c]) => c === code)?.[1] ?? null;
}

/** The text every screen shows for a failure of the call nodes; '' for none. */
export function callNodeErrorText(error: string, translate: (key: CallNodeErrorKey) => string): string {
  if (!error) return '';
  const key = callNodeErrorKey(error);
  return key ? translate(key) : error;
}
