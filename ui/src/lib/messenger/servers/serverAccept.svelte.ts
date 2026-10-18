// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The question before a server is added (internal/messenger-wire.md,
// «Принятие сервера»): every path that adds one — the card of a server, a
// link of a person or a group with the card of its server, a contact card
// with a server, a link pasted into «Новый чат» or «Новая группа», a QR —
// goes through `acceptServer`, which shows `ServerAcceptDialog` (mounted
// once per page, beside ConfirmHost) and adds nothing unless the user says
// «Добавить». What the dialog says is `serverSummary`: the server's name,
// whether it is Veydan's, and what it brings.

import { messengerApi, type ServerPreview } from '../api';

type Translate = (key: string, params?: Record<string, string>) => string;

/** A host of an address, without its scheme and path: what the dialog names. */
export function hostOf(url: string): string {
  return url.replace(/^[a-z]+:\/\//i, '').replace(/[/?#].*$/, '');
}

/** The name the dialog gives a server: its own, «Veydan», or its relay's host. */
export function serverName(s: ServerPreview, tr: Translate): string {
  if (s.is_veydan) return tr('msg_srv_veydan_name');
  return s.name.trim() || hostOf(s.relay);
}

/**
 * What a server brings, in the words of the dialog: its relay, its media,
 * its calls and its push, each on its own («без …» for what it has not).
 */
export function serverFacts(s: ServerPreview, tr: Translate): string[] {
  return [
    tr('msg_srv_relay', { host: hostOf(s.relay) }),
    s.media ? tr('msg_srv_media', { host: hostOf(s.media) }) : tr('msg_srv_media_none'),
    s.call_node ? tr('msg_srv_calls', { addr: s.call_node.replace(/#.*$/, '') }) : tr('msg_srv_calls_none'),
    tr(s.push === 'veydan' ? 'msg_srv_push_veydan' : 'msg_srv_push_off'),
  ];
}

/** One line of the dialog: «S1 — не Veydan: реле …, медиа …, звонки …, без push» or «серверы Veydan: …». */
export function serverSummary(s: ServerPreview, tr: Translate): string {
  const whose = s.is_veydan ? tr('msg_srv_is_veydan') : tr('msg_srv_not_veydan');
  return `${serverName(s, tr)} — ${whose}: ${serverFacts(s, tr).join(', ')}`;
}

interface Pending {
  servers: ServerPreview[];
  resolve: (yes: boolean) => void;
}

class ServerAcceptState {
  current = $state<Pending | null>(null);

  answer(yes: boolean): void {
    const pending = this.current;
    this.current = null;
    pending?.resolve(yes);
  }
}

export const serverAcceptState = new ServerAcceptState();

/** Ask whether to add these servers; true when the user said «Добавить». A second question answers the first «no». */
export function askServers(servers: ServerPreview[]): Promise<boolean> {
  serverAcceptState.answer(false);
  return new Promise((resolve) => {
    serverAcceptState.current = { servers, resolve };
  });
}

/**
 * The server of a card, mine after this: one I have is not asked about
 * again; another is added only when the user says yes. Its id, or `null`
 * when the user said no.
 */
export async function acceptServer(s: ServerPreview): Promise<string | null> {
  if (s.known) return s.id;
  if (!(await askServers([s]))) return null;
  return messengerApi.servers.accept(s.link);
}
