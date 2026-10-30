// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { MessengerChat, MessengerMessage } from '../api';
import type { TimelineItem } from '../content/types';
import { addUnits, chosenText, forwardable, forwardOutcome, ForwardSessions, forwardTarget, selectUnits, toggleUnit, unitRange, type Unit } from './select';
import dialogSource from '../content/ForwardDialog.svelte?raw';

const msg = (over: Partial<MessengerMessage> = {}) =>
  ({ id: 'm1', status: 'received', content_type: 'text', deleted: false, review: 'ok', text: 'hi', ...over }) as MessengerMessage;

const chat = (over: Partial<MessengerChat> = {}) =>
  ({ id: 'dm:aa', kind: 'dm', can_send: true, archived: false, is_contact: true, mode: 'full_chat', ...over }) as MessengerChat;

const run = { first: true, last: true, showAuthor: false, showAvatar: false, indent: false };
const bubble = (m: MessengerMessage): TimelineItem => ({ type: 'bubble', id: m.id, message: m, ...run });
const album = (id: string, messages: MessengerMessage[]): TimelineItem => ({ type: 'album', id, variant: 'visual', messages, ...run });
const unit = (key: string, n = 1): Unit => ({ key, ids: Array.from({ length: n }, (_, i) => (n === 1 ? key : `${key}.${i}`)) });

describe('forwardable', () => {
  it('takes a text, a file and a card, sent or received', () => {
    expect(forwardable(msg())).toBe(true);
    expect(forwardable(msg({ status: 'sent', content_type: 'media' }))).toBe(true);
    expect(forwardable(msg({ content_type: 'contact' }))).toBe(true);
  });

  it('leaves what was removed, what has not left and what failed', () => {
    expect(forwardable(msg({ deleted: true }))).toBe(false);
    for (const status of ['queued', 'failed', 'uploading', 'paused'] as const) expect(forwardable(msg({ status }))).toBe(false);
    expect(forwardable(msg({ id: 'local:1', status: 'sent' }))).toBe(false);
  });

  it('leaves the lines of the app, calls, cards of the bot and types it does not know', () => {
    for (const content_type of ['system', 'call', 'control', 'bot_card', 'unknown']) expect(forwardable(msg({ content_type }))).toBe(false);
    expect(forwardable(msg({ bot_card: { v: 1 } as unknown as MessengerMessage['bot_card'] }))).toBe(false);
  });

  it('leaves what waits for a moderator or was turned down', () => {
    expect(forwardable(msg({ review: 'pending' }))).toBe(false);
    expect(forwardable(msg({ review: 'rejected' }))).toBe(false);
    expect(forwardable(msg({ review: undefined }))).toBe(true);
  });
});

describe('forwardTarget', () => {
  it('takes a contact and a group of mine where I write', () => {
    expect(forwardTarget(chat())).toBe(true);
    expect(forwardTarget(chat({ id: 'dm:aa@srv' }))).toBe(true);
    expect(forwardTarget(chat({ id: 'group:g1', kind: 'group', is_contact: false, mode: 'full_chat' }))).toBe(true);
  });

  it('leaves the bot, a stranger, a request, the archive, the blocked and what I cannot write in', () => {
    expect(forwardTarget(chat({ bot: true }))).toBe(false);
    expect(forwardTarget(chat({ is_contact: false, mode: 'first_contact' }))).toBe(false);
    expect(forwardTarget(chat({ is_contact: false, mode: 'request_received' }))).toBe(false);
    expect(forwardTarget(chat({ archived: true }))).toBe(false);
    expect(forwardTarget(chat({ mode: 'blocked', can_send: false }))).toBe(false);
    expect(forwardTarget(chat({ banned: true }))).toBe(false);
    expect(forwardTarget(chat({ id: 'group:g1', kind: 'group', can_send: false }))).toBe(false);
  });

  it('leaves a channel of Communities', () => {
    expect(forwardTarget(chat({ id: 'channel:c1', kind: 'channel' }))).toBe(false);
    expect(forwardTarget(chat({ id: 'channel:c1', kind: 'group' }))).toBe(false);
  });
});

describe('selectUnits', () => {
  it('makes one unit of a bubble and one of an album, its parts that may go, and none of the rest', () => {
    const items: TimelineItem[] = [
      { type: 'day', id: 'd', at: 0 },
      bubble(msg({ id: 'a' })),
      { type: 'system', id: 's', messages: [msg({ id: 's', content_type: 'system' })] },
      bubble(msg({ id: 'gone', deleted: true })),
      album('al', [msg({ id: 'p1', content_type: 'media' }), msg({ id: 'p2', content_type: 'media', status: 'failed' }), msg({ id: 'p3', content_type: 'media' })]),
      album('dead', [msg({ id: 'q1', content_type: 'media', status: 'uploading' })]),
    ];
    expect(selectUnits(items)).toEqual([{ key: 'a', ids: ['a'] }, { key: 'al', ids: ['p1', 'p3'] }]);
  });
});

describe('ticking', () => {
  it('ticks and unticks a unit, an album whole', () => {
    const al = unit('al', 3);
    const on = toggleUnit([], al);
    expect(on).toEqual({ chosen: ['al.0', 'al.1', 'al.2'], refused: false });
    expect(toggleUnit(on.chosen, al).chosen).toEqual([]);
  });

  it('refuses what passes the limit, an album that does not fit whole too', () => {
    expect(toggleUnit(['x', 'y'], unit('z'), 2)).toEqual({ chosen: ['x', 'y'], refused: true });
    expect(toggleUnit(['x'], unit('al', 2), 2)).toEqual({ chosen: ['x'], refused: true });
    // Unticking is never refused.
    expect(toggleUnit(['x', 'y'], unit('x'), 2)).toEqual({ chosen: ['y'], refused: false });
  });

  it('ticks a range as far as the limit holds', () => {
    const units = [unit('a'), unit('b'), unit('c'), unit('d')];
    expect(unitRange(units, 'b', 'd').map((u) => u.key)).toEqual(['b', 'c', 'd']);
    expect(unitRange(units, 'd', 'b').map((u) => u.key)).toEqual(['d', 'c', 'b']);
    expect(unitRange(units, 'a', 'gone')).toEqual([]);
    expect(addUnits(['b'], unitRange(units, 'b', 'd'), 3)).toEqual({ chosen: ['b', 'c', 'd'], refused: false });
    expect(addUnits([], units, 3)).toEqual({ chosen: ['a', 'b', 'c'], refused: true });
  });

  it('copies the words of what is chosen in the order of the chat', () => {
    const list = [msg({ id: 'a', text: 'one' }), msg({ id: 'b', text: null }), msg({ id: 'c', text: ' three ' }), msg({ id: 'd', text: 'no' })];
    expect(chosenText(list, ['c', 'a', 'b'])).toBe('one\n\nthree');
  });
});

describe('forwardOutcome', () => {
  it('tells the chats that got all of it from those that got less', () => {
    expect(forwardOutcome([
      { target: 'dm:a', sent: [] },
      { target: 'group:g', sent: [], skipped: [{ message_id: 'm', reason: 'err.forward_media_unavailable' }] },
      { target: 'dm:b', sent: [], error: 'dm_blocked' },
    ])).toEqual({
      done: ['dm:a'],
      short: [
        { target: 'group:g', error: null, skipped: ['err.forward_media_unavailable'] },
        { target: 'dm:b', error: 'dm_blocked', skipped: [] },
      ],
    });
  });
});

describe('the openings of the sheet of a forward', () => {
  it('leave a send of A, finished after its sheet was closed and B opened, out of B', () => {
    const s = new ForwardSessions();
    s.turn(); // A opens
    const a = s.start();
    expect(s.live(a)).toBe(true);
    s.turn(); // A is closed while it sends
    s.turn(); // B opens
    const b = s.start();
    expect(s.live(a)).toBe(false);
    expect(s.live(b)).toBe(true);
  });

  it('leave a send whose sheet was only closed out of what it would close', () => {
    const s = new ForwardSessions();
    s.turn();
    const a = s.start();
    s.turn();
    expect(s.live(a)).toBe(false);
  });

  it('are what the dialog asks before it closes, calls `onforwarded`, shows an error or frees the button', () => {
    const send = dialogSource.slice(dialogSource.indexOf('async function send()'), dialogSource.indexOf('</script>'));
    const guard = send.indexOf('if (!sessions.live(opening)) return;');
    expect(guard).toBeGreaterThan(0);
    expect(send.indexOf('open = false')).toBeGreaterThan(guard);
    expect(send.indexOf('onforwarded?.()')).toBeGreaterThan(guard);
    expect(send).toContain('if (sessions.live(opening)) busy = false;');
    expect(send).toMatch(/if \(!sessions\.live\(opening\)\) toast\.error\(explain\(e\)[^;]*;\s*else error = explain\(e\);/);
    expect(dialogSource).toContain('if (open !== wasOpen) sessions.turn();');
  });

  it('show «Скрыть отправителя» with the switch of the house, not a checkbox under its class', () => {
    expect(dialogSource).not.toContain('<label class="toggle">');
    expect(dialogSource).toMatch(/<button class="toggle" class:on=\{hideSender\} role="switch"/);
  });
});
