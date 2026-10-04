// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { MessengerMessage } from '../api';
import { albumKey, buildTimeline, MAX_ALBUM, RUN_GAP_SECS } from './timeline';
import type { TimelineItem } from './types';

const ME = 'ab'.repeat(32);
const ANN = '1a'.repeat(32);
const BOB = '2b'.repeat(32);
/** Noon, local time: a day does not change by itself within the tests. */
const NOON = Math.floor(new Date(2026, 8, 29, 12, 0, 0).getTime() / 1000);
const DAY = 86_400;

let n = 0;
function msg(from: string, at: number, extra: Partial<MessengerMessage> = {}): MessengerMessage {
  return {
    id: `m${n++}`, chat_id: 'c', direction: from === ME ? 'out' : 'in', status: 'sent', content_type: 'text', text: 'x',
    sender_pubkey: from, reply_to: null, created_at: at, edited_at: null, deleted: false, failure_reason: null, media: null, ...extra,
  };
}
const sys = (at: number) => msg(ANN, at, { content_type: 'system', text: 'group_joined' });
const pic = (from: string, at: number, batch: string | null, kind = 'image') =>
  msg(from, at, { content_type: 'media', text: null, media: { name: 'a.png', kind, ...(batch ? { batch } : {}) } });

/** `d` day, `s` system (with how many lines), `b` bubble, `a` album (with how many); `<` first of a run, `>` last. */
function shape(items: TimelineItem[]): string {
  return items
    .map((i) => {
      if (i.type === 'day') return 'd';
      if (i.type === 'system') return `s${i.messages.length}`;
      const run = `${i.first ? '<' : ''}${i.last ? '>' : ''}`;
      return i.type === 'album' ? `a${i.messages.length}${run}` : `b${run}`;
    })
    .join(' ');
}

describe('buildTimeline', () => {
  it('is empty for an empty chat', () => {
    expect(buildTimeline([], { many: false })).toEqual([]);
  });

  it('joins messages of one author into runs', () => {
    const list = [msg(ANN, NOON), msg(ANN, NOON + 10), msg(ANN, NOON + 20), msg(ME, NOON + 30), msg(ANN, NOON + 40)];
    expect(shape(buildTimeline(list, { many: false }))).toBe('d b< b b> b<> b<>');
  });

  it('breaks a run after a pause', () => {
    const list = [msg(ANN, NOON), msg(ANN, NOON + RUN_GAP_SECS), msg(ANN, NOON + 2 * RUN_GAP_SECS + 1)];
    expect(shape(buildTimeline(list, { many: false }))).toBe('d b< b> b<>');
  });

  it('breaks a run at a new day and at a system line', () => {
    const list = [msg(ANN, NOON), msg(ANN, NOON + DAY), msg(ANN, NOON + DAY + 1), sys(NOON + DAY + 2), msg(ANN, NOON + DAY + 3)];
    const items = buildTimeline(list, { many: true });
    expect(shape(items)).toBe('d b<> d b< b> s1 b<>');
    expect(items.filter((i) => i.type === 'day').map((i) => i.id).length).toBe(2);
  });

  it('shows the name first and the picture last, for others, in a chat of many', () => {
    const list = [msg(ANN, NOON), msg(ANN, NOON + 1), msg(ANN, NOON + 2), msg(ME, NOON + 3), msg(ME, NOON + 4), msg(BOB, NOON + 5)];
    const flags = (many: boolean) =>
      buildTimeline(list, { many }).flatMap((i) => (i.type === 'bubble' ? [`${i.showAuthor ? 'n' : '-'}${i.showAvatar ? 'p' : '-'}${i.indent ? 'i' : '-'}`] : []));
    expect(flags(true)).toEqual(['n-i', '--i', '-pi', '---', '---', 'npi']);
    expect(flags(false)).toEqual(['---', '---', '---', '---', '---', '---']);
  });

  it('makes an album of what was sent by one action', () => {
    const list = [pic(ANN, NOON, 'b1'), pic(ANN, NOON + 1, 'b1', 'video'), pic(ANN, NOON + 2, 'b1'), msg(ANN, NOON + 3)];
    const items = buildTimeline(list, { many: true });
    expect(shape(items)).toBe('d a3< b>');
    const album = items[1];
    expect(album.type === 'album' && album.variant).toBe('visual');
    expect(album.type === 'album' && album.messages.map((m) => m.id)).toEqual(list.slice(0, 3).map((m) => m.id));
    expect(album.type === 'album' && [album.showAuthor, album.showAvatar]).toEqual([true, false]);
  });

  it('shows files sent together as one album, and pictures sent with them in the same one', () => {
    const list = [
      pic(ME, NOON, 'b1', 'file'), pic(ME, NOON, 'b1', 'audio'), pic(ME, NOON, 'b1', 'file'),
      pic(ME, NOON + 1, 'b1'), pic(ME, NOON + 1, 'b1'),
    ];
    const items = buildTimeline(list, { many: false });
    expect(shape(items)).toBe('d a5<>');
    expect(items.flatMap((i) => (i.type === 'album' ? [i.variant] : []))).toEqual(['mixed']);
    const files = buildTimeline(list.slice(0, 3), { many: false });
    expect(files.flatMap((i) => (i.type === 'album' ? [i.variant] : []))).toEqual(['files']);
  });

  it('shows a picture alone as a picture, and a file alone as a message', () => {
    const items = buildTimeline([pic(ANN, NOON, null), pic(ANN, NOON + 1, null, 'file'), pic(ANN, NOON + 2, null, 'voice')], { many: false });
    expect(shape(items)).toBe('d a1< b b>');
    const quoted = { ...pic(ANN, NOON, null), reply_to: { id: 'x', sender_pubkey: ME, text: 'q' } };
    expect(shape(buildTimeline([quoted], { many: false })), 'a reply keeps its quote').toBe('d b<>');
  });

  it('puts no more than ten into one album', () => {
    const list = Array.from({ length: 23 }, (_, i) => pic(ANN, NOON + i, 'big'));
    const sizes = buildTimeline(list, { many: false }).flatMap((i) => (i.type === 'album' ? [i.messages.length] : []));
    expect(sizes).toEqual([MAX_ALBUM, MAX_ALBUM, 3]);
  });

  it('guesses no album', () => {
    // The same second, the same author, and nothing that says "one action".
    expect(shape(buildTimeline([pic(ANN, NOON, null), pic(ANN, NOON, null)], { many: false }))).toBe('d a1< a1>');
    expect(shape(buildTimeline([pic(ANN, NOON, 'b1'), pic(ANN, NOON, 'b2')], { many: false }))).toBe('d a1< a1>');
    expect(shape(buildTimeline([pic(ANN, NOON, 'b1'), pic(BOB, NOON, 'b1')], { many: false }))).toBe('d a1<> a1<>');
    expect(shape(buildTimeline([pic(ANN, NOON, 'b1'), msg(ANN, NOON + 1), pic(ANN, NOON + 2, 'b1')], { many: false }))).toBe('d a1< b a1>');
    expect(shape(buildTimeline([pic(ANN, NOON, 'b1'), pic(ANN, NOON + DAY, 'b1')], { many: false }))).toBe('d a1<> d a1<>');
    expect(shape(buildTimeline([pic(ANN, NOON, 'b1')], { many: false })), 'one picture is a picture').toBe('d a1<>');

    expect(albumKey(pic(ANN, NOON, 'b1', 'file'))).toBe('b1');
    expect(albumKey(pic(ANN, NOON, 'b1', 'voice'))).toBeNull();
    expect(albumKey(pic(ANN, NOON, 'b1', 'circle'))).toBeNull();
    expect(albumKey({ ...pic(ANN, NOON, 'b1'), deleted: true })).toBeNull();
    expect(albumKey(msg(ANN, NOON, { media: { batch: 'b1', kind: 'image' } })), 'a text is not a picture').toBeNull();
    expect(albumKey(pic(ANN, NOON, 'b1'))).toBe('b1');
  });

  it('folds system lines that come in a row', () => {
    expect(shape(buildTimeline([sys(NOON), sys(NOON + 1)], { many: true }))).toBe('d s1 s1');
    expect(shape(buildTimeline([sys(NOON), sys(NOON + 1), sys(NOON + 2), sys(NOON + 3)], { many: true }))).toBe('d s4');
    expect(shape(buildTimeline([sys(NOON), sys(NOON + 1), msg(ANN, NOON + 2), sys(NOON + 3)], { many: true }))).toBe('d s1 s1 b<> s1');
    expect(shape(buildTimeline([sys(NOON), sys(NOON + 1), sys(NOON + DAY), sys(NOON + DAY + 1)], { many: true }))).toBe('d s1 s1 d s1 s1');
  });

  it('gives every item its own id', () => {
    const list = [sys(NOON), sys(NOON + 1), sys(NOON + 2), msg(ANN, NOON + 3), pic(ANN, NOON + 4, 'b'), pic(ANN, NOON + 5, 'b'), msg(ME, NOON + DAY)];
    const ids = buildTimeline(list, { many: true }).map((i) => i.id);
    expect(new Set(ids).size).toBe(ids.length);
  });
});
