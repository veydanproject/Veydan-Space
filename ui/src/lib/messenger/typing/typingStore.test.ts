// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// «печатает…» is a deadline: it goes by the clock when nothing more comes,
// at once when the author's message comes, and a repeat keeps it on; the
// clock runs only while somebody types.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { typingStore } from './typingStore.svelte';
import { typingText } from './labels';

const BOT = 'b0'.repeat(32);
const EVA = 'e0'.repeat(32);
const ARTEM = 'a0'.repeat(32);
const LEV = 'c0'.repeat(32);
const T0 = 1_800_000_000;

const now = () => Date.now() / 1000;
const dm = (chat_id: string, peer: string, ahead = 6) => ({ name: 'dm.typing', payload: { chat_id, peer, until: now() + ahead } });
const inGroup = (group_id: string, member: string, ahead = 6) => ({ name: 'group.typing', payload: { group_id, member, until: now() + ahead } });
const underPost = (post_id: string, author: string, ahead = 6) => ({ name: 'channel.typing', payload: { community_id: 'c', post_id, author, until: now() + ahead } });
const message = (chat_id: string, sender_pubkey: string, extra: Record<string, unknown> = {}) => ({
  name: 'dm.message',
  payload: { chat_id, message: { id: 'm', chat_id, direction: 'in', sender_pubkey }, ...extra },
});

describe('who is typing', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(T0 * 1000);
    typingStore.reset();
  });
  afterEach(() => {
    typingStore.reset();
    vi.useRealTimers();
  });

  it('goes when its deadline passes, though nothing more comes', () => {
    typingStore.handleEvent(dm('dm:bot', BOT));
    typingStore.handleEvent(inGroup('g', EVA));
    typingStore.handleEvent(underPost('p', EVA));
    expect(typingStore.inChat('dm:bot')).toBe(true);
    expect(typingStore.inGroup('g')).toEqual([EVA]);
    expect(typingStore.underPost('p')).toEqual([EVA]);
    vi.advanceTimersByTime(5000);
    expect(typingStore.inChat('dm:bot')).toBe(true);
    vi.advanceTimersByTime(1000);
    expect(typingStore.inChat('dm:bot')).toBe(false);
    expect(typingStore.inGroup('g')).toEqual([]);
    expect(typingStore.underPost('p')).toEqual([]);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('stays while it is said again, and the clock stops once nobody types', () => {
    expect(vi.getTimerCount()).toBe(0);
    typingStore.handleEvent(inGroup('g', EVA));
    expect(vi.getTimerCount()).toBe(1);
    for (let i = 0; i < 5; i++) {
      vi.advanceTimersByTime(4000);
      typingStore.handleEvent(inGroup('g', EVA));
    }
    expect(typingStore.inGroup('g')).toEqual([EVA]);
    expect(vi.getTimerCount()).toBe(1);
    vi.advanceTimersByTime(7000);
    expect(typingStore.inGroup('g')).toEqual([]);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('keeps several, the first to start first, each by its own deadline', () => {
    typingStore.handleEvent(inGroup('g', EVA, 3));
    typingStore.handleEvent(inGroup('g', ARTEM, 6));
    typingStore.handleEvent(inGroup('g', LEV, 6));
    typingStore.handleEvent(inGroup('other', LEV, 6));
    typingStore.handleEvent(inGroup('g', EVA, 6));
    expect(typingStore.inGroup('g')).toEqual([EVA, ARTEM, LEV]);
    typingStore.handleEvent(underPost('p1', EVA, 2));
    typingStore.handleEvent(underPost('p1', ARTEM, 6));
    typingStore.handleEvent(underPost('p2', LEV, 6));
    vi.advanceTimersByTime(3000);
    expect(typingStore.underPost('p1')).toEqual([ARTEM]);
    expect(typingStore.underPost('p2')).toEqual([LEV]);
    expect(typingStore.inGroup('g')).toEqual([EVA, ARTEM, LEV]);
    expect(typingStore.inGroup('other')).toEqual([LEV]);
  });

  it('ends at once when a live message of its author comes', () => {
    typingStore.handleEvent(dm('dm:bot', BOT));
    typingStore.handleEvent(inGroup('g', EVA));
    typingStore.handleEvent(inGroup('g', ARTEM));
    // History, my own message and somebody else's end nothing.
    typingStore.handleEvent(message('dm:bot', BOT, { historical: true }));
    typingStore.handleEvent({ name: 'dm.message', payload: { chat_id: 'dm:bot', message: { chat_id: 'dm:bot', direction: 'out', sender_pubkey: BOT } } });
    typingStore.handleEvent(message('dm:other', BOT));
    expect(typingStore.inChat('dm:bot')).toBe(true);
    typingStore.handleEvent(message('dm:bot', BOT));
    expect(typingStore.inChat('dm:bot')).toBe(false);
    typingStore.handleEvent(message('group:g', EVA));
    expect(typingStore.inGroup('g')).toEqual([ARTEM]);
    typingStore.handleEvent(message('group:g', ARTEM));
    expect(typingStore.inGroup('g')).toEqual([]);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('ends under a post when the thread says the comment came', () => {
    typingStore.handleEvent(underPost('p', EVA));
    typingStore.handleEvent(underPost('p', ARTEM));
    typingStore.clear({ post: 'p' }, EVA);
    expect(typingStore.underPost('p')).toEqual([ARTEM]);
    typingStore.clear({ post: 'other' }, ARTEM);
    expect(typingStore.underPost('p')).toEqual([ARTEM]);
  });

  it('takes a deadline already passed as stopped, and cuts one too far ahead', () => {
    typingStore.handleEvent(dm('dm:bot', BOT));
    typingStore.handleEvent(dm('dm:bot', BOT, -1));
    expect(typingStore.inChat('dm:bot')).toBe(false);
    typingStore.handleEvent(inGroup('g', EVA, 3600));
    vi.advanceTimersByTime(16_000);
    expect(typingStore.inGroup('g')).toEqual([]);
  });

  it('ignores an event that does not say who, where or until when', () => {
    typingStore.handleEvent({ name: 'dm.typing', payload: { chat_id: 'dm:bot', peer: BOT } });
    typingStore.handleEvent({ name: 'group.typing', payload: { group_id: 'g', until: now() + 6 } });
    typingStore.handleEvent({ name: 'channel.typing', payload: { author: EVA, until: now() + 6 } });
    typingStore.handleEvent({ name: 'dm.typing', payload: null });
    expect(typingStore.chats).toEqual({});
    expect(typingStore.groups).toEqual({});
    expect(typingStore.posts).toEqual({});
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe('the words of «печатает…»', () => {
  const tr = (key: string, vars: Record<string, string> = {}) => `${key}(${Object.entries(vars).map(([k, v]) => `${k}=${v}`).join(',')})`;

  it('names one or two, and counts the rest', () => {
    expect(typingText([], tr, 'ru')).toBe('msg_typing()');
    expect(typingText(['Ева'], tr, 'ru')).toBe('msg_typing_1(name=Ева)');
    expect(typingText(['Ева', 'Артём'], tr, 'ru')).toBe('msg_typing_2(a=Ева,b=Артём)');
    expect(typingText(['Ева', 'Артём', 'Лев'], tr, 'ru')).toBe('msg_typing_more_one(a=Ева,b=Артём,n=1)');
    expect(typingText(['Ева', 'Артём', 'Лев', 'Ия'], tr, 'ru')).toBe('msg_typing_more_few(a=Ева,b=Артём,n=2)');
    expect(typingText(['A', 'B', 'C', 'D', 'E', 'F', 'G'], tr, 'en')).toBe('msg_typing_more(a=A,b=B,n=5)');
  });
});
