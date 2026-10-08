// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The call nodes of the browser preview (`pnpm dev` without Tauri): the
// project's two nodes, a volunteer's from the registry, and whatever is
// added by a link. Never used inside the app. A link whose token is
// `used` answers as a spent invitation does; any other token is taken.

import type { CallNodeInfo, CallNodesView, CallTrust } from '../generated/calls';

const ID = /^[0-9a-f]{64}$/;

function project(addr: string, id: string, region: string): CallNodeInfo {
  return {
    reference: `${addr}#${id}`, id, addr, class: 'project', source: 'manifest', mine: false, has_key: false, region,
    caps: ['stun', 'turn', 'turn-tcp', 'turn-tls', 'sfu', 'simulcast', 'cascade'], health: 'unknown', used: true,
  };
}

let trust: CallTrust = 'any';
let checkedAt: number | null = null;
let mine: CallNodeInfo[] = [];
const others: CallNodeInfo[] = [
  project('108.61.171.68:8443', 'fda09da75199c4e04601a9df309710fab15cd7b2b806ca3b2bbab202582a6dca', 'eu'),
  project('149.28.37.154:8443', '68367a61a90c3efafa3a0d4aec481be7c8c21f266864871ecbe9e8ff139bfb03', 'us'),
  {
    reference: `198.51.100.23:443#${'5e'.repeat(32)}`, id: '5e'.repeat(32), addr: '198.51.100.23:443', class: 'volunteer',
    source: 'registry', mine: false, has_key: false, region: 'eu', caps: ['stun', 'turn', 'sfu'], load: 35, health: 'active', used: true,
  },
];

function allowed(n: CallNodeInfo): boolean {
  if (n.mine) return true;
  if (trust === 'own_only') return false;
  return trust === 'any' || n.class !== 'volunteer';
}

function view(): CallNodesView {
  const nodes = [...mine, ...others.filter((o) => !mine.some((m) => m.id === o.id))].map((n) => ({ ...n, used: allowed(n) }));
  return { trust, nodes, registry: true, registry_checked_at: checkedAt, registry_paused: false };
}

/** The node a text names, as the runtime would read it; a refusal code otherwise. */
export function demoNodeOf(text: string): { id: string; addr: string; token: string | null } | string {
  const t = text.trim();
  const link = /^veydan:\/\/([a-z]+(?:-[a-z]+)*)\/([0-9a-fA-F]+)\?(.*)$/.exec(t);
  if (link) {
    if (link[1] !== 'call-node') return 'call_node_link_type';
    const p = new URLSearchParams(link[3]);
    const addr = p.get('a') ?? '';
    if (!ID.test(link[2].toLowerCase()) || !/^[0-9.]+:\d{1,5}$/.test(addr)) return 'call_node_invalid';
    return { id: link[2].toLowerCase(), addr, token: p.get('t') };
  }
  const ref = /^([0-9.]+:\d{1,5})#([0-9a-fA-F]{64})$/.exec(t);
  return ref ? { id: ref[2].toLowerCase(), addr: ref[1], token: null } : 'call_node_invalid';
}

/** Whether the node `id` is among mine (the card of a link says so): for a link with an invitation, only by one exchanged before. */
export function demoNodeIsMine(id: string, invitation = false): boolean {
  return mine.some((n) => n.id === id && (!invitation || n.source === 'device'));
}

export function demoCallNodeMocks(): Record<string, (args?: Record<string, unknown>) => unknown> {
  return {
    messenger_call_nodes_list: (a) => {
      if (a?.probe) {
        let i = 0;
        for (const n of [...mine, ...others]) {
          if (!allowed(n)) continue;
          n.rtt_ms = 30 + 25 * i++;
          n.health = n.class === 'volunteer' && (n.load ?? 0) > 90 ? 'degraded' : 'active';
          n.load ??= 5 * i;
        }
      }
      if (trust === 'any' && checkedAt === null) checkedAt = Math.floor(Date.now() / 1000);
      return view();
    },
    messenger_call_nodes_add: (a) => {
      const node = demoNodeOf(String(a?.link ?? ''));
      if (typeof node === 'string') throw { code: 'other', message: node };
      if (node.token === 'used') throw { code: 'other', message: 'call_node_bad_invite' };
      const key = String(a?.key ?? '').trim();
      mine = [
        {
          reference: `${node.addr}#${node.id}`, id: node.id, addr: node.addr, class: 'own', source: node.token ? 'device' : 'setting',
          mine: true, has_key: Boolean(node.token || key), label: node.token ? 'Demo node' : undefined, caps: ['stun', 'turn', 'sfu'],
          rtt_ms: 18, load: 2, health: 'active', private: Boolean(node.token), used: true,
          added_at: node.token ? Math.floor(Date.now() / 1000) : undefined,
        },
        ...mine.filter((n) => n.id !== node.id),
      ];
      return view();
    },
    messenger_call_nodes_remove: (a) => {
      const id = String(a?.id ?? '').toLowerCase();
      if (!mine.some((n) => n.id === id)) throw { code: 'other', message: 'call_node_unknown' };
      mine = mine.filter((n) => n.id !== id);
      return view();
    },
    messenger_call_nodes_set_trust: (a) => {
      trust = a?.trust === 'own_only' || a?.trust === 'project_and_own' ? a.trust : 'any';
      return view();
    },
    messenger_call_nodes_refresh: () => {
      if (trust === 'any') checkedAt = Math.floor(Date.now() / 1000);
      return view();
    },
  };
}
