// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Who may write to me, how long a request waits and whom I blocked, as
// the privacy store keeps them: read apart from the switches, sent whole,
// and the blocked read once for a burst of events.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const b = (peer: string) => ({ peer, npub: peer ? `npub1${peer}` : '', name: peer.toUpperCase() });

const api = vi.hoisted(() => ({
  dmSettings: vi.fn(async () => ({ dm_from: 'anyone', keep_days: 7 })),
  dmSettingsSet: vi.fn(async (_s: unknown) => {}),
  blocked: vi.fn(async () => [b('aa'), b(''), b('bb')]),
  action: vi.fn(async (_peer: string, _action: string) => {}),
  privacyGet: vi.fn(async () => ({ read_receipts: true, presence: true })),
  privacySet: vi.fn(async (read_receipts: boolean, presence: boolean) => ({ read_receipts, presence })),
}));
vi.mock('../api', () => ({
  messengerApi: {
    moderation: { dmSettings: api.dmSettings, dmSettingsSet: api.dmSettingsSet, blocked: api.blocked },
    dm: { action: api.action },
    privacy: { get: api.privacyGet, set: api.privacySet },
  },
  messengerError: (e: unknown) => (e instanceof Error ? e.message : String(e)),
}));

const chats = vi.hoisted(() => ({ loadChats: vi.fn(async () => {}), reloadWindow: vi.fn(async () => {}) }));
vi.mock('../chats/chatStore.svelte', () => ({ chatStore: chats }));

const presence = vi.hoisted(() => ({ setSharing: vi.fn(), load: vi.fn(async () => {}) }));
vi.mock('../presence/presenceStore.svelte', () => ({ presenceStore: presence }));

const { privacyStore } = await import('./privacyStore.svelte');

beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockClear();
  chats.loadChats.mockClear();
  privacyStore.dm = null;
  privacyStore.blocked = null;
  privacyStore.dmError = '';
  privacyStore.blockedError = '';
});

afterEach(() => {
  vi.useRealTimers();
});

describe('who may write to me', () => {
  it('is read apart from the switches, with the blocked named, empty keys left out', async () => {
    await privacyStore.loadDm();
    expect(privacyStore.dm).toEqual({ dm_from: 'anyone', keep_days: 7 });
    expect(privacyStore.blocked).toEqual([b('aa'), b('bb')]);
    expect(api.privacyGet).not.toHaveBeenCalled();
  });

  it('keeps the banned flag of a person the network removed', async () => {
    api.blocked.mockResolvedValueOnce([{ ...b('aa'), banned: true }, b('bb')] as never);
    await privacyStore.loadDm();
    expect(privacyStore.blocked?.map((x) => x.banned)).toEqual([true, undefined]);
  });

  it('keeps the words of a runtime that lacks the commands', async () => {
    api.dmSettings.mockRejectedValueOnce(new Error('unknown command'));
    await privacyStore.loadDm();
    expect(privacyStore.dm).toBeNull();
    expect(privacyStore.dmError).toBe('unknown command');
  });

  it('sends the settings whole and reads them back', async () => {
    await privacyStore.loadDm();
    api.dmSettings.mockResolvedValueOnce({ dm_from: 'contacts', keep_days: 7 });
    await privacyStore.setDmFrom('contacts');
    expect(api.dmSettingsSet).toHaveBeenLastCalledWith({ dm_from: 'contacts', keep_days: 7 });
    expect(privacyStore.dm).toEqual({ dm_from: 'contacts', keep_days: 7 });
    api.dmSettings.mockResolvedValueOnce({ dm_from: 'contacts', keep_days: 1 });
    await privacyStore.setKeepDays(1);
    expect(api.dmSettingsSet).toHaveBeenLastCalledWith({ dm_from: 'contacts', keep_days: 1 });
    expect(privacyStore.dm?.keep_days).toBe(1);
    expect(privacyStore.dmBusy).toBe(false);
  });

  it('changes nothing before the settings were read, and words a refusal', async () => {
    await privacyStore.setKeepDays(3);
    expect(api.dmSettingsSet).not.toHaveBeenCalled();
    await privacyStore.loadDm();
    api.dmSettingsSet.mockRejectedValueOnce(new Error('dm_from'));
    await privacyStore.setDmFrom('contacts');
    expect(privacyStore.dmError).toBe('dm_from');
    expect(privacyStore.dm).toEqual({ dm_from: 'anyone', keep_days: 7 });
  });
});

describe('the blocked', () => {
  it('are read again after an unblock, and the chats with them', async () => {
    await privacyStore.loadDm();
    api.blocked.mockResolvedValueOnce([b('bb')]);
    await privacyStore.unblock('aa');
    expect(api.action).toHaveBeenCalledWith('aa', 'unblock');
    expect(privacyStore.blocked).toEqual([b('bb')]);
    expect(privacyStore.unblocking).toBeNull();
    expect(chats.loadChats).toHaveBeenCalled();
  });

  it('stay as they were when the unblock is refused', async () => {
    await privacyStore.loadDm();
    api.action.mockRejectedValueOnce(new Error('dm_unknown'));
    await privacyStore.unblock('aa');
    expect(privacyStore.blocked).toEqual([b('aa'), b('bb')]);
    expect(privacyStore.blockedError).toBe('dm_unknown');
    expect(privacyStore.unblocking).toBeNull();
  });

  it('are read once for a burst of relationship events, and not before they were read at all', async () => {
    vi.useFakeTimers();
    privacyStore.handleEvent({ name: 'dm.relationship', payload: {} });
    await vi.runAllTimersAsync();
    expect(api.blocked).not.toHaveBeenCalled();
    await privacyStore.loadDm();
    api.blocked.mockClear();
    api.blocked.mockResolvedValue([b('cc')]);
    privacyStore.handleEvent({ name: 'dm.relationship', payload: {} });
    privacyStore.handleEvent({ name: 'dm.request', payload: {} });
    privacyStore.handleEvent({ name: 'dm.relationship', payload: {} });
    expect(api.blocked).not.toHaveBeenCalled();
    await vi.runAllTimersAsync();
    expect(api.blocked).toHaveBeenCalledTimes(1);
    expect(privacyStore.blocked).toEqual([b('cc')]);
  });
});
