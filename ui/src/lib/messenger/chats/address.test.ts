// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A private chat («Частный · S1») is addressed with its server: a text, a
// file, a recording, a relationship action and a call of it name
// `<peer>@<server>`, never the bare peer, which is the home chat over the
// home relays. The home chat and a group stay as they were.

import { beforeEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => {
  const sent = { id: 'm1', chat_id: 'x', created_at: 1 };
  return {
    sendText: vi.fn(async () => sent),
    sendFile: vi.fn(async () => sent),
    sendRecording: vi.fn(async () => sent),
    action: vi.fn(async () => ({})),
    start: vi.fn(async () => null),
  };
});
vi.mock('../api', () => ({
  messengerApi: {
    dm: { sendText: api.sendText, action: api.action },
    media: { sendFile: api.sendFile, sendRecording: api.sendRecording },
    chats: { list: vi.fn(async () => []), messages: vi.fn(async () => []), markRead: vi.fn(async () => {}), open: vi.fn() },
    profiles: { request: vi.fn(async () => {}) },
    calls: { start: api.start },
  },
}));
vi.mock('../store.svelte', () => ({ messengerStore: { refreshContacts: vi.fn(async () => {}), status: { runtime: { session_active: true } } } }));

const { chatAddress, parseAddress } = await import('./address');
const { chatStore } = await import('./chatStore.svelte');
const { targetOf } = await import('../calls/dial.svelte');
const { callStore } = await import('../calls/callStore.svelte');

const PEER = 'ab'.repeat(32);
const S1 = '0123456789abcdef';
const chat = (over: Record<string, unknown>) => ({
  id: `dm:${PEER}`, kind: 'dm', peer_pubkey: PEER, peer_npub: null, title: 'B', picture: null, is_contact: true, is_muted: false,
  unread: 0, last_message_at: null, last_preview: null, pinned: false, archived: false, mode: 'full_chat', can_send: true, ...over,
}) as never;
const privateChat = chat({ id: `dm:${PEER}@${S1}`, server_id: S1 });
const homeChat = chat({});

describe('the address of a chat', () => {
  it('names the server of a private chat, the peer of the home one, the id of a group', () => {
    expect(chatAddress(privateChat)).toBe(`${PEER}@${S1}`);
    expect(chatAddress(homeChat)).toBe(PEER);
    expect(chatAddress(chat({ id: 'group:g', kind: 'group', peer_pubkey: null }))).toBe('group:g');
    // A view without `server_id` still names the server its id carries.
    expect(chatAddress(chat({ id: `dm:${PEER}@${S1}` }))).toBe(`${PEER}@${S1}`);
    expect(parseAddress(`dm:${PEER}@${S1}`)).toEqual({ peer: PEER, server: S1 });
    expect(parseAddress(PEER)).toEqual({ peer: PEER, server: null });
  });
});

describe('what a private chat sends', () => {
  beforeEach(() => {
    for (const f of Object.values(api)) f.mockClear();
    chatStore.chats = [privateChat, homeChat];
    chatStore.activeId = `dm:${PEER}@${S1}`;
  });

  it('a text, a file and a recording go to <peer>@<server>', async () => {
    await chatStore.send('hi');
    await chatStore.sendFile('/tmp/a.png');
    await chatStore.sendRecording({ kind: 'voice' } as never);
    await chatStore.sendTo(privateChat, 'fwd');
    expect(api.sendText.mock.calls.map((c) => (c as unknown[])[0])).toEqual([`${PEER}@${S1}`, `${PEER}@${S1}`]);
    expect((api.sendFile.mock.calls[0] as unknown[])[0]).toBe(`${PEER}@${S1}`);
    expect((api.sendRecording.mock.calls[0] as unknown[])[0]).toBe(`${PEER}@${S1}`);
  });

  it('accept and block act on the private chat, the home chat on the bare peer', async () => {
    await chatStore.act(`dm:${PEER}@${S1}`, 'block');
    chatStore.chats = [privateChat, homeChat];
    await chatStore.act(`dm:${PEER}`, 'accept');
    expect(api.action.mock.calls.map((c) => (c as unknown[]).slice(0, 2))).toEqual([[`${PEER}@${S1}`, 'block'], [PEER, 'accept']]);
  });

  it('a call dials <peer>@<server>', async () => {
    const target = targetOf(privateChat);
    expect(target.peer).toBe(PEER);
    expect(target.address).toBe(`${PEER}@${S1}`);
    await callStore.dial(target.address, 'audio');
    expect((api.start.mock.calls[0] as unknown[])[0]).toBe(`${PEER}@${S1}`);
    expect(targetOf(homeChat).address).toBe(PEER);
  });
});
