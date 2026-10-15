// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// My invitations as the store keeps them: read with the pause, a new one
// in the list at once, a revoke read back, an event a reason to read
// again; and the way in by someone's link or code, which sends a request
// with the secret only to a person who is not me and not blocked.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => {
  const view = (id: string, over: Record<string, unknown> = {}) => ({
    id, kind: 'link', secret: id, label: '', created_at: 1, used: 0, revoked: false, ...over,
  });
  return {
    view,
    inviteList: vi.fn(async () => [view('a'), view('b')]),
    inviteStatus: vi.fn(async () => ({ paused_until: null as number | null })),
    inviteCreate: vi.fn(async (p: { kind: string }) => view('new', { kind: p.kind })),
    inviteRevoke: vi.fn(async (_id: string) => {}),
    inspect: vi.fn(async (_links: string[]) => [] as unknown[]),
    action: vi.fn(async (_peer: string, _action: string, _extra?: unknown) => ({})),
  };
});
vi.mock('../api', () => ({
  messengerApi: {
    moderation: { inviteList: api.inviteList, inviteStatus: api.inviteStatus, inviteCreate: api.inviteCreate, inviteRevoke: api.inviteRevoke },
    links: { inspect: api.inspect },
    dm: { action: api.action },
  },
  messengerError: (e: unknown) => (e instanceof Error ? e.message : String(e)),
}));

const chats = vi.hoisted(() => ({ loadChats: vi.fn(async () => {}) }));
vi.mock('../chats/chatStore.svelte', () => ({ chatStore: chats }));
vi.mock('../store.svelte', () => ({ messengerStore: { refreshContacts: vi.fn(async () => {}) } }));

const { inviteStore } = await import('./inviteStore.svelte');

const PEER = 'cd'.repeat(32);
const contact = (over: Record<string, unknown> = {}) => ({
  kind: 'contact', link: 'veydan://contact/npub1x', pubkey: PEER, npub: 'npub1x', name: 'Gleb', picture: null, nip05: null,
  is_me: false, is_contact: false, blocked: false, ...over,
});

beforeEach(() => {
  inviteStore.reset();
  for (const fn of Object.values(api)) if (typeof fn === 'function' && 'mockClear' in fn) fn.mockClear();
  chats.loadChats.mockClear();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('my invitations', () => {
  it('are read with the pause', async () => {
    api.inviteStatus.mockResolvedValueOnce({ paused_until: 2_000 });
    await inviteStore.load();
    expect(inviteStore.list?.map((v) => v.id)).toEqual(['a', 'b']);
    expect(inviteStore.pausedAt).toBe(2_000);
    expect(inviteStore.pausedUntil(1_999)).toBe(2_000);
    expect(inviteStore.pausedUntil(2_000)).toBeNull();
  });

  it('keep the words of a runtime that lacks the commands, and nothing of a list', async () => {
    api.inviteList.mockRejectedValueOnce(new Error('unknown command'));
    await inviteStore.load();
    expect(inviteStore.list).toBeNull();
    expect(inviteStore.error).toBe('unknown command');
  });

  it('take a new one at the top at once', async () => {
    await inviteStore.load();
    const v = await inviteStore.create({ kind: 'code', label: '' });
    expect(api.inviteCreate).toHaveBeenCalledWith({ kind: 'code', label: '' });
    expect(v.kind).toBe('code');
    expect(inviteStore.list?.map((x) => x.id)).toEqual(['new', 'a', 'b']);
  });

  it('keep one entry when the runtime answers with an id already listed', async () => {
    await inviteStore.load();
    api.inviteCreate.mockResolvedValueOnce(api.view('b', { kind: 'qr' }));
    await inviteStore.create({ kind: 'qr', label: '' });
    expect(inviteStore.list?.map((x) => x.id)).toEqual(['b', 'a']);
    expect(inviteStore.list?.[0].kind).toBe('qr');
  });

  it('show a revoke at once and read the list back', async () => {
    await inviteStore.load();
    api.inviteList.mockResolvedValueOnce([api.view('a', { revoked: true }), api.view('b')]);
    await inviteStore.revoke('a');
    expect(api.inviteRevoke).toHaveBeenCalledWith('a');
    expect(inviteStore.list?.[0].revoked).toBe(true);
    expect(inviteStore.revoking).toBeNull();
  });

  it('keep the list when a revoke is refused, and word it', async () => {
    await inviteStore.load();
    api.inviteRevoke.mockRejectedValueOnce(new Error('invite_unknown'));
    await inviteStore.revoke('a');
    expect(inviteStore.error).toBe('invite_unknown');
    expect(inviteStore.list?.every((v) => !v.revoked)).toBe(true);
  });

  it('take the pause from its event, and read again once for a burst', async () => {
    vi.useFakeTimers();
    inviteStore.handleEvent({ name: 'dm.invite_paused', payload: { until: 5_000 } });
    expect(inviteStore.pausedAt).toBe(5_000);
    // An `until` that is no number is no pause: what was said stays.
    inviteStore.handleEvent({ name: 'dm.invite_paused', payload: { until: 'soon' } });
    expect(inviteStore.pausedAt).toBe(5_000);
    await vi.runAllTimersAsync();
    // Nothing was read yet: an event alone does not start a read.
    expect(api.inviteList).not.toHaveBeenCalled();
    await inviteStore.load();
    api.inviteList.mockClear();
    inviteStore.handleEvent({ name: 'dm.relationship', payload: {} });
    inviteStore.handleEvent({ name: 'history.synced', payload: {} });
    inviteStore.handleEvent({ name: 'dm.relationship', payload: {} });
    await vi.runAllTimersAsync();
    expect(api.inviteList).toHaveBeenCalledTimes(1);
  });

  it('of a past session are not filled by an answer that comes late', async () => {
    let answer!: (v: unknown) => void;
    api.inviteList.mockReturnValueOnce(new Promise((r) => (answer = r)) as never);
    const loading = inviteStore.load();
    inviteStore.reset();
    answer([api.view('late')]);
    await loading;
    expect(inviteStore.list).toBeNull();
  });
});

describe('the way in by a link', () => {
  it('with a secret sends a request with it, then reads the chats again', async () => {
    api.inspect.mockResolvedValueOnce([contact({ invite: 'q7Vd0xKp3LmN8sRt2WyZaA' })]);
    const out = await inviteStore.join(' veydan://contact/npub1x?i=q7Vd0xKp3LmN8sRt2WyZaA ');
    expect(api.inspect).toHaveBeenCalledWith(['veydan://contact/npub1x?i=q7Vd0xKp3LmN8sRt2WyZaA']);
    expect(api.action).toHaveBeenCalledWith(PEER, 'request', { invite: 'q7Vd0xKp3LmN8sRt2WyZaA' });
    expect(chats.loadChats).toHaveBeenCalled();
    expect(out).toEqual({ ok: true, pubkey: PEER, name: 'Gleb', invited: true });
  });

  it('without a secret sends nothing: the chat opens as with any key', async () => {
    api.inspect.mockResolvedValueOnce([contact()]);
    const out = await inviteStore.join('veydan://contact/npub1x');
    expect(api.action).not.toHaveBeenCalled();
    expect(out).toEqual({ ok: true, pubkey: PEER, name: 'Gleb', invited: false });
  });

  it('sends nothing to my own link, to someone I blocked, or for what is not a person', async () => {
    api.inspect.mockResolvedValueOnce([contact({ is_me: true, invite: 'x'.repeat(22) })]);
    expect(await inviteStore.join('veydan://contact/me')).toEqual({ ok: false, reason: 'is_me' });
    api.inspect.mockResolvedValueOnce([contact({ blocked: true, invite: 'x'.repeat(22) })]);
    expect(await inviteStore.join('veydan://contact/them')).toEqual({ ok: false, reason: 'blocked' });
    api.inspect.mockResolvedValueOnce([{ kind: 'group', link: 'veydan://group/x' }]);
    expect(await inviteStore.join('veydan://group/x')).toEqual({ ok: false, reason: 'not_contact' });
    expect(api.action).not.toHaveBeenCalled();
  });
});

describe('a request with a code', () => {
  it('carries the first message, trimmed, with the secret', async () => {
    await inviteStore.request(PEER, '482913', '  Hello, it is Gleb  ');
    expect(api.action).toHaveBeenCalledWith(PEER, 'request', { invite: '482913', text: 'Hello, it is Gleb' });
  });

  it('carries no empty message', async () => {
    await inviteStore.request(PEER, '482913', '   ');
    expect(api.action).toHaveBeenCalledWith(PEER, 'request', { invite: '482913' });
  });
});
