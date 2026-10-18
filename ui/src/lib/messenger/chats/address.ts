// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the runtime's commands name a chat by (internal/messenger-wire.md,
// «Частный чат»): a group by its chat id, a chat between two by its peer —
// `<hex>` the home chat, `<hex>@<server id>` the private chat on that
// server. The bare peer of a private chat would name the home chat, and
// what was meant for the private server would go over the home relays.

import type { MessengerChat } from '../api';

type Addressed = Pick<MessengerChat, 'id' | 'kind' | 'peer_pubkey'> & { server_id?: string | null };

/** The server of a direct chat's id (`dm:<hex>@<server>`), if it names one. */
function serverOfId(id: string): string | null {
  if (!id.startsWith('dm:')) return null;
  const at = id.indexOf('@');
  return at > 0 ? id.slice(at + 1) || null : null;
}

/** The address of `chat` for the runtime; `null` when it names nobody. */
export function chatAddress(chat: Addressed | null | undefined): string | null {
  if (!chat) return null;
  if (chat.kind === 'group') return chat.id;
  const peer = chat.peer_pubkey;
  if (!peer) return null;
  if (peer.includes('@')) return peer;
  const server = chat.server_id || serverOfId(chat.id);
  return server ? `${peer}@${server}` : peer;
}

/** The peer and the server an address (or a direct chat id) names: the demo reads it as the runtime does. */
export function parseAddress(addr: string): { peer: string; server: string | null } {
  const bare = addr.trim().replace(/^dm:/, '');
  const at = bare.indexOf('@');
  return at < 0 ? { peer: bare, server: null } : { peer: bare.slice(0, at), server: bare.slice(at + 1) || null };
}
