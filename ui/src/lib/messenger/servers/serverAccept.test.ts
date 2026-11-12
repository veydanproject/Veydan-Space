// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// «Добавить сервер?»: what the dialog says of a server, in both built-in
// languages, and that nothing is added unless the user says «Добавить» —
// a server this device has is not asked about at all.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import dialogSource from './ServerAcceptDialog.svelte?raw';
import listSource from './PrivateServers.svelte?raw';
import desktopSource from '../entry/desktop.ts?raw';
import pageSource from '../pages/MessengerPage.svelte?raw';
import frameSource from '../mobile/MobileFrame.svelte?raw';
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

const { acceptServer, askServers, serverAcceptState, serverInfo, serverName, serverRows } = await import('./serverAccept.svelte');

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
  access: 'open',
  invite: null,
};
const VEYDAN: ServerPreview = {
  ...S1, link: 'veydan://server/0123456789abcdef?u=wss%3A%2F%2Fnode-1.veydan.net&k=&p=off&n=', id: '0123456789abcdef', name: '',
  relay: 'wss://node-1.veydan.net', media: null, call_node: null, push_url: null, push: 'off', is_veydan: true, access: 'open',
};

describe('what the dialog says of a server', () => {
  it('lists every server a chat on it goes through, a line each', () => {
    expect(serverRows(S1, tr('ru')).map((r) => `${r.label}: ${r.value}`)).toEqual([
      'Доступ: открытый: любой с картой', 'Сообщения: relay.s1.example', 'Файлы: media.s1.example', 'Звонки: 203.0.113.7:8443', 'Уведомления: через Veydan',
    ]);
    expect(serverRows(S1, tr('en')).map((r) => r.label)).toEqual(['Access', 'Messages', 'Files', 'Calls', 'Notifications']);
  });

  it('says what a server has not, as not there', () => {
    const rows = serverRows(VEYDAN, tr('ru'));
    expect(rows.map((r) => [r.value, !!r.none])).toEqual([
      ['открытый: любой с картой', false], ['node-1.veydan.net', false], ['нет', true], ['напрямую', true], ['нет', true],
    ]);
  });

  it('says the access mode of every server in the first row', () => {
    const access = (a: ServerPreview['access'] | undefined) => serverRows({ ...S1, access: a as ServerPreview['access'] }, tr('ru'))[0];
    expect(access('open')).toEqual({ label: 'Доступ', value: 'открытый: любой с картой' });
    expect(access('team').value).toBe('по приглашению: только участники');
    expect(access('channel').value).toBe('канал: читать могут все, пишут авторы');
    expect(access(undefined).value).toBe('открытый: любой с картой');
    expect(serverRows({ ...S1, access: 'team' }, tr('en'))[0].value).toBe('by invitation: members only');
  });

  it('notes the invitation of a card only when it carries one', () => {
    expect(dialogSource).toMatch(/some\(\(x\) => x\.invite\)/);
    expect(dialogSource).toContain("$t('msg_srv_invite_note')");
    expect(tr('en')('msg_srv_invite_note')).toContain('invitation');
  });

  it('shows «Нет доступа» and the code prompt for a denied server of the list', () => {
    expect(listSource).toContain('{#if s.denied}');
    expect(listSource).toContain("$t('msg_srv_no_access')");
    expect(listSource).toContain('messengerApi.servers.join(');
    for (const code of ['msg_srv_invite_invalid', 'msg_srv_invite_used', 'msg_srv_invite_expired', 'msg_srv_join_failed']) {
      expect(listSource).toMatch(new RegExp(code === 'msg_srv_join_failed' ? code : `server_${code.slice(8)}`));
      expect(tr('ru')(code)).not.toBe('');
    }
  });

  it('opens «Частный сервер» for a server and shuts it', () => {
    serverInfo.show(S1);
    expect(serverInfo.current?.relay).toBe(S1.relay);
    serverInfo.close();
    expect(serverInfo.current).toBeNull();
  });

  it('names a server without a name by the host of its relay', () => {
    expect(serverName({ ...S1, name: '  ' }, tr('ru'))).toBe('relay.s1.example');
  });

  it('is a dialog with «Добавить» and «Отмена», its texts in the dictionary', () => {
    expect(dialogSource).toContain("$t('msg_srv_accept_title')");
    expect(dialogSource).toContain("$t('common_add')");
    expect(dialogSource).toContain("$t('common_cancel')");
    for (const key of ['msg_srv_accept_title', 'msg_srv_accept_lead', 'msg_srv_accept_note']) {
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

describe('where the dialog is', () => {
  const mounts = (src: string, tag: string) => src.split(`<${tag} />`).length - 1;
  const dialogs = ['ServerAcceptDialog', 'ServerInfoDialog', 'PrivateInviteDialog'];

  it('is one for the window on a computer: an overlay of the module, so the core\'s Settings ask it too, not the page of the chats (ROO-1075)', () => {
    for (const d of dialogs) {
      expect(desktopSource.match(new RegExp(`component: ${d}, place: 'window'`, 'g'))?.length).toBe(1);
      expect(mounts(pageSource, d)).toBe(0);
      expect(pageSource).not.toContain(`import ${d} `);
    }
  });

  it('stays in the frame of each screen on a phone', () => {
    for (const d of dialogs) expect(mounts(frameSource, d)).toBe(1);
  });
});
