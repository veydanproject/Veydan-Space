// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { MessengerGroup, MessengerGroupMember, MessengerGroupRequest } from '../../api';
import {
  bulkTried, cameAt, claimedApart, commonGroups, filterRequests, firstLine, keepSelected, matchRequest, runBulk, selectAll,
  shortNpub, sortRequests, toggleSelected, waited, waitedText, type BulkState,
} from './requests';

const req = (requester: string, received_at: number, note = '', created_at = received_at): MessengerGroupRequest =>
  ({ requester, npub: `npub1${requester}`, note, created_at, received_at });

describe('sortRequests', () => {
  const list = [req('b', 200), req('a', 100), req('c', 300), req('d', 200)];
  it('puts the newest first, or the oldest', () => {
    expect(sortRequests(list, 'newest').map((r) => r.requester)).toEqual(['c', 'b', 'd', 'a']);
    expect(sortRequests(list, 'oldest').map((r) => r.requester)).toEqual(['a', 'b', 'd', 'c']);
  });
  it('leaves the list it was given as it was', () => {
    sortRequests(list, 'newest');
    expect(list.map((r) => r.requester)).toEqual(['b', 'a', 'c', 'd']);
  });
});

describe('matchRequest and filterRequests', () => {
  const r = req('ab12', 1, 'I am from the translators community');
  const words = { name: 'Irina Lebedeva', nip05: 'irina@nostr.example' };
  it('finds by the name, the key, the address and the note, any case', () => {
    expect(matchRequest('irina', r, words)).toBe(true);
    expect(matchRequest('NPUB1AB', r, words)).toBe(true);
    expect(matchRequest('ab12', r, words)).toBe(true);
    expect(matchRequest('nostr.example', r, words)).toBe(true);
    expect(matchRequest('Translators', r, words)).toBe(true);
  });
  it('wants every word typed', () => {
    expect(matchRequest('irina community', r, words)).toBe(true);
    expect(matchRequest('irina boris', r, words)).toBe(false);
  });
  it('an empty search keeps everything', () => {
    const list = [r, req('cd', 2)];
    expect(filterRequests(list, '  ', () => words)).toHaveLength(2);
    expect(filterRequests(list, 'translators', (x) => (x === r ? words : { name: 'Oleg', nip05: null }))).toEqual([r]);
  });
});

describe('waited and waitedText', () => {
  it('takes the largest whole unit', () => {
    expect(waited(1000, 1030)).toBeNull();
    expect(waited(1000, 1000 + 5 * 60 + 59)).toEqual({ unit: 'minute', n: 5 });
    expect(waited(0, 2 * 3600 + 10)).toEqual({ unit: 'hour', n: 2 });
    expect(waited(0, 3 * 86400 + 7200)).toEqual({ unit: 'day', n: 3 });
  });
  it('a request dated ahead of my clock has waited nothing', () => {
    expect(waited(2000, 1000)).toBeNull();
  });
  it('speaks the units of the language', () => {
    expect(waitedText({ unit: 'hour', n: 2 }, 'en')).toBe('2 hr');
    expect(waitedText({ unit: 'hour', n: 2 }, 'ru')).toBe('2 ч');
    expect(waitedText({ unit: 'day', n: 3 }, 'ru')).toMatch(/^3 дн/);
  });
});

describe('claimedApart', () => {
  it('only a date more than ten minutes from the moment it came', () => {
    expect(claimedApart(req('a', 10_000, '', 10_000 - 600))).toBe(false);
    expect(claimedApart(req('a', 10_000, '', 10_000 - 601))).toBe(true);
    expect(claimedApart(req('a', 10_000, '', 10_000 + 3600))).toBe(true);
  });
});

describe('cameAt', () => {
  const now = new Date(2026, 9, 17, 15, 0);
  const at = (d: Date) => Math.floor(d.getTime() / 1000);
  const day = (k: 'today' | 'yesterday') => (k === 'today' ? 'Today' : 'Yesterday');
  it('names today and yesterday, dates the rest', () => {
    expect(cameAt(at(new Date(2026, 9, 17, 14, 5)), now, 'en-GB', day)).toBe('Today, 14:05');
    expect(cameAt(at(new Date(2026, 9, 16, 9, 12)), now, 'en-GB', day)).toBe('Yesterday, 09:12');
    expect(cameAt(at(new Date(2026, 8, 29, 14, 5)), now, 'en-GB', day)).toMatch(/^29 Sept?, 14:05$/);
    expect(cameAt(at(new Date(2025, 8, 29, 14, 5)), now, 'en-GB', day)).toMatch(/2025/);
  });
});

describe('commonGroups', () => {
  const member = (pubkey: string): MessengerGroupMember => ({ pubkey, role: 'member', muted: false, trusted: false, joined_at: 0, is_me: false });
  const group = (id: string, membership: MessengerGroup['membership'], people: string[]) =>
    ({ id, membership, members: people.map(member) }) as unknown as MessengerGroup;
  it('counts the other groups I am in where they are', () => {
    const groups = [group('here', 'joined', ['x']), group('g1', 'joined', ['x', 'y']), group('g2', 'joined', ['y']), group('g3', 'left', ['x'])];
    expect(commonGroups(groups, 'here', 'x')).toBe(1);
    expect(commonGroups(groups, 'here', 'y')).toBe(2);
    expect(commonGroups(groups, 'here', 'z')).toBe(0);
  });
});

describe('firstLine', () => {
  it('the first line with words, cut to its length', () => {
    expect(firstLine('\n  Translator.  \nKazakh, Russian')).toBe('Translator.');
    expect(firstLine('')).toBeNull();
    expect(firstLine(null)).toBeNull();
    expect(firstLine('a'.repeat(200), 10)).toBe(`${'a'.repeat(9)}…`);
  });
});

describe('the ticks', () => {
  const shown = [req('a', 1), req('b', 2)];
  it('toggles one', () => {
    expect(toggleSelected(['a'], 'b')).toEqual(['a', 'b']);
    expect(toggleSelected(['a', 'b'], 'a')).toEqual(['b']);
  });
  it('«select all» ticks the list shown and unticks it when all of it is ticked', () => {
    expect(selectAll(['z'], shown)).toEqual(['z', 'a', 'b']);
    expect(selectAll(['z', 'a', 'b'], shown)).toEqual(['z']);
    expect(selectAll([], [])).toEqual([]);
  });
  it('drops the ticks of requests that are gone', () => {
    expect(keepSelected(['a', 'gone', 'b'], shown)).toEqual(['a', 'b']);
  });
});

describe('runBulk', () => {
  it('acts one after another, keeps going past a failure and tells each step', async () => {
    const order: string[] = [];
    let inFlight = 0;
    const steps: BulkState[] = [];
    const end = await runBulk('approve', ['a', 'b', 'c', 'd'], async (who) => {
      inFlight += 1;
      expect(inFlight).toBe(1);
      await Promise.resolve();
      order.push(who);
      inFlight -= 1;
      if (who === 'b' || who === 'd') throw new Error(`no ${who}`);
    }, (e) => (e as Error).message, (s) => steps.push(s));
    expect(order).toEqual(['a', 'b', 'c', 'd']);
    expect(end).toEqual({ kind: 'approve', total: 4, done: 2, failed: [{ requester: 'b', error: 'no b' }, { requester: 'd', error: 'no d' }], running: false });
    expect(steps[0]).toMatchObject({ done: 0, running: true });
    expect(steps.map(bulkTried)).toEqual([0, 1, 2, 3, 4, 4]);
    expect(steps.at(-1)?.running).toBe(false);
  });
  it('nothing to do ends at once', async () => {
    const end = await runBulk('ban', [], async () => {}, String);
    expect(end).toEqual({ kind: 'ban', total: 0, done: 0, failed: [], running: false });
  });
});

describe('shortNpub', () => {
  it('keeps the head and the tail of a long key, a short one whole', () => {
    expect(shortNpub(`npub1${'a'.repeat(52)}xyz789`)).toBe('npub1aaaaa…xyz789');
    expect(shortNpub('npub1short')).toBe('npub1short');
  });
});
