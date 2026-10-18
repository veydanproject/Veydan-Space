// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// «Добавить сервер?»: what the dialog says of a server, in both built-in
// languages, and that nothing is added unless the user says «Добавить» —
// a server this device has is not asked about at all.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import dialogSource from './ServerAcceptDialog.svelte?raw';
import { messengerTranslations } from '../i18n';
import type { ServerPreview } from '../api';

const accepted: string[] = [];
vi.mock('../api', () => ({
  messengerApi: {
    servers: {
      accept: async (link: string) => {
        accepted.push(link);
        return 'e06676e9354ae2e2';
      },
    },
  },
}));

const { acceptServer, askServers, serverAcceptState, serverFacts, serverName, serverSummary } = await import('./serverAccept.svelte');

type Lang = keyof typeof messengerTranslations;
const tr = (lang: Lang) => (key: string, params: Record<string, string> = {}) => {
  const text = (messengerTranslations[lang] as Record<string, string>)[key];
  if (text === undefined) throw new Error(`no ${lang} string ${key}`);
  return text.replace(/\{(\w+)\}/g, (_, k: string) => params[k] ?? `{${k}}`);
};

const S1: ServerPreview = {
  link: 'veydan://server/e06676e9354ae2e2?u=wss%3A%2F%2Frelay.s1.example&k=key&m=https%3A%2F%2Fmedia.s1.example&c=203.0.113.7%3A8443%23ab&p=veydan&n=S1',
  id: 'e06676e9354ae2e2',
  name: 'S1',
  relay: 'wss://relay.s1.example',
  media: 'https://media.s1.example',
  call_node: `203.0.113.7:8443#${'ab'.repeat(32)}`,
  push_url: 'https://push.s1.example',
  push: 'veydan',
  known: false,
  is_veydan: false,
};
const VEYDAN: ServerPreview = {
  ...S1, link: 'veydan://server/0123456789abcdef?u=wss%3A%2F%2Fnode-1.veydan.net&k=&p=off&n=', id: '0123456789abcdef', name: '',
  relay: 'wss://node-1.veydan.net', media: null, call_node: null, push_url: null, push: 'off', is_veydan: true,
};

describe('what the dialog says of a server', () => {
  it('names it, says it is not Veydan, and lists what it brings', () => {
    expect(serverSummary(S1, tr('ru'))).toBe(
      'S1 — не Veydan: реле relay.s1.example, медиа media.s1.example, звонки 203.0.113.7:8443, push через сеть Veydan',
    );
    expect(serverSummary(S1, tr('en'))).toBe(
      'S1 — not Veydan: relay relay.s1.example, media media.s1.example, calls 203.0.113.7:8443, push through the Veydan network',
    );
  });

  it('says «серверы Veydan» for a relay of the Veydan network, and what it has not', () => {
    expect(serverSummary(VEYDAN, tr('ru'))).toBe('Veydan — серверы Veydan: реле node-1.veydan.net, без медиа, без узла звонков, без push');
    expect(serverFacts(VEYDAN, tr('en'))).toEqual(['relay node-1.veydan.net', 'no media', 'no call node', 'no push']);
  });

  it('names a server without a name by the host of its relay', () => {
    expect(serverName({ ...S1, name: '  ' }, tr('ru'))).toBe('relay.s1.example');
  });

  it('is a dialog with «Добавить» and «Отмена», its texts in the dictionary', () => {
    expect(dialogSource).toContain("$t('msg_srv_accept_title')");
    expect(dialogSource).toContain("$t('msg_srv_accept_yes')");
    expect(dialogSource).toContain("$t('common_cancel')");
    for (const key of ['msg_srv_accept_title', 'msg_srv_accept_lead', 'msg_srv_accept_note', 'msg_srv_accept_yes']) {
      expect(tr('ru')(key)).not.toBe('');
      expect(tr('en')(key)).not.toBe('');
    }
    expect(tr('ru')('msg_srv_accept_lead')).toBe('Будут добавлены серверы:');
  });
});

describe('adding a server', () => {
  beforeEach(() => {
    accepted.length = 0;
    serverAcceptState.answer(false);
  });

  it('asks first, and adds nothing on «Отмена»', async () => {
    const asked = acceptServer(S1);
    await Promise.resolve();
    expect(serverAcceptState.current?.servers).toEqual([S1]);
    serverAcceptState.answer(false);
    expect(await asked).toBeNull();
    expect(accepted).toEqual([]);
  });

  it('adds it on «Добавить», by its card', async () => {
    const asked = acceptServer(S1);
    await Promise.resolve();
    serverAcceptState.answer(true);
    expect(await asked).toBe('e06676e9354ae2e2');
    expect(accepted).toEqual([S1.link]);
  });

  it('does not ask about a server this device has', async () => {
    expect(await acceptServer({ ...S1, known: true })).toBe(S1.id);
    expect(serverAcceptState.current).toBeNull();
    expect(accepted).toEqual([]);
  });

  it('answers an open question «no» when a second one comes', async () => {
    const first = askServers([S1]);
    const second = askServers([VEYDAN]);
    expect(await first).toBe(false);
    expect(serverAcceptState.current?.servers).toEqual([VEYDAN]);
    serverAcceptState.answer(true);
    expect(await second).toBe(true);
  });
});
