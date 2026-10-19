// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The question before a server is added (internal/messenger-wire.md,
// «Принятие сервера»): every path that adds one — the card of a server, a
// link of a person or a group with the card of its server, a contact card
// with a server, a link pasted into «Новый чат» or «Новая группа», a QR —
// goes through `acceptServer`, which shows `ServerAcceptDialog` (mounted
// once per page, beside ConfirmHost) and adds nothing unless the user says
// «Добавить». What the dialog says is the server's name,
// whether it is private or Veydan's, and its lines (`serverRows`).

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

/** What a server is made of, as a card, a link or the list of mine tells it. */
export interface ServerParts {
  name: string | null;
  relay: string;
  media: string | null;
  call_node: string | null;
  push: string;
  is_veydan?: boolean;
}

/** One line of «what this server is»: what it carries, and where. */
export interface ServerRow {
  label: string;
  value: string;
  /** Not there: «нет», «напрямую». */
  none?: boolean;
}

/**
 * Every server a chat on it goes through, a line each: the messages (its
 * relay), the files (its media), the calls (its node) and the
 * notifications (the Veydan gateway, or none).
 */
export function serverRows(s: ServerParts, tr: Translate): ServerRow[] {
  return [
    { label: tr('msg_srv_row_messages'), value: hostOf(s.relay) },
    s.media ? { label: tr('msg_srv_row_files'), value: hostOf(s.media) } : { label: tr('msg_srv_row_files'), value: tr('msg_srv_row_none'), none: true },
    s.call_node
      ? { label: tr('msg_srv_row_calls'), value: s.call_node.replace(/#.*$/, '') }
      : { label: tr('msg_srv_row_calls'), value: tr('msg_srv_row_calls_direct'), none: true },
    s.push === 'veydan'
      ? { label: tr('msg_srv_row_push'), value: tr('msg_srv_row_push_veydan') }
      : { label: tr('msg_srv_row_push'), value: tr('msg_srv_row_none'), none: true },
  ];
}

class ServerInfoState {
  /** The server «ⓘ» was pressed for; `null` when the window is shut. */
  current = $state<ServerParts | null>(null);

  show(s: ServerParts): void {
    this.current = s;
  }

  close(): void {
    this.current = null;
  }
}

/** «Частный сервер» (ServerInfoDialog): who runs it, and what it is made of. */
export const serverInfo = new ServerInfoState();
