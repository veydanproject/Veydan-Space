// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { MessengerChat } from '../api';
import { captionOf, directTarget, draftOf, filesLine, kindOf, MAX_SHARE_TARGETS, shareTarget } from './shareIn';

const chat = (over: Partial<MessengerChat>): MessengerChat =>
  ({ id: 'dm:a', kind: 'dm', title: 'A', can_send: true, archived: false, banned: false, bot: false, is_contact: true, mode: 'full_chat', ...over }) as MessengerChat;

describe('a share from another app', () => {
  it('opens the composer with the subject above the text, nothing for files alone', () => {
    expect(draftOf({ text: 'see this', subject: null })).toBe('see this');
    expect(draftOf({ text: 'https://x.example/p', subject: 'A page' })).toBe('A page\nhttps://x.example/p');
    expect(draftOf({ text: '  ', subject: null })).toBe('');
    expect(draftOf({ text: null, subject: null })).toBe('');
  });

  it('captions the files sent to several chats with the text and the comment', () => {
    expect(captionOf({ text: 'see', subject: null }, '')).toBe('see');
    expect(captionOf({ text: 'see', subject: null }, ' for you ')).toBe('see\nfor you');
    expect(captionOf({ text: null, subject: null }, 'for you')).toBe('for you');
    expect(captionOf({ text: null, subject: null }, '  ')).toBeUndefined();
  });

  it('goes straight to the chat the system sheet chose, when the share may go there', () => {
    const chats = [chat({ id: 'dm:a' }), chat({ id: 'group:g', kind: 'group' }), chat({ id: 'dm:b', can_send: false })];
    expect(directTarget({ target: 'dm:a' }, chats)?.id).toBe('dm:a');
    expect(directTarget({ target: 'group:g' }, chats)?.id).toBe('group:g');
    // A chat that takes nothing now, one that is gone, none chosen: the chooser.
    expect(directTarget({ target: 'dm:b' }, chats)).toBeNull();
    expect(directTarget({ target: 'dm:zzz' }, chats)).toBeNull();
    expect(directTarget({ target: null }, chats)).toBeNull();
  });

  it('offers the chats a forward may go to, ten at most', () => {
    expect(shareTarget(chat({}))).toBe(true);
    expect(shareTarget(chat({ archived: true }))).toBe(false);
    expect(shareTarget(chat({ kind: 'dm', mode: 'request' as MessengerChat['mode'] }))).toBe(false);
    expect(MAX_SHARE_TARGETS).toBe(10);
  });

  it('names the first files and counts the rest', () => {
    const f = (name: string) => ({ path: `/x/${name}`, name, mime: null, size: 1 });
    expect(filesLine([f('a.jpg'), f('b.jpg')])).toEqual({ names: ['a.jpg', 'b.jpg'], more: 0 });
    expect(filesLine([f('a'), f('b'), f('c'), f('d'), f('e')])).toEqual({ names: ['a', 'b', 'c'], more: 2 });
    expect(filesLine([])).toEqual({ names: [], more: 0 });
  });

  it('tells the kind of a file by its mime', () => {
    expect(kindOf({ mime: 'image/jpeg', name: 'a' })).toBe('image');
    expect(kindOf({ mime: 'video/mp4', name: 'a' })).toBe('video');
    expect(kindOf({ mime: 'audio/ogg', name: 'a' })).toBe('audio');
    expect(kindOf({ mime: 'application/pdf', name: 'a' })).toBe('file');
    expect(kindOf({ mime: null, name: 'a' })).toBe('file');
  });
});
