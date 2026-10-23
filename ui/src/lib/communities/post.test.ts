// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { MdBlock } from '$lib/messenger/seams/shared';
import { dropFirstLine, editedAt, linesOf, postTitle, recordingCard } from './post';
import { isRecording, mediaOf, type FeedItemView } from './api';

describe('postTitle', () => {
  it('takes the first line of plain words as the title, the rest as the text', () => {
    expect(postTitle('Что нового в 5.3\nКаналы получили разделы.\nИ ещё.', null)).toEqual({ title: 'Что нового в 5.3', rest: 'Каналы получили разделы.\nИ ещё.' });
  });

  it('keeps the marks of plain words as they are', () => {
    expect(postTitle('# not a heading\n**bold**', undefined)).toEqual({ title: '# not a heading', rest: '**bold**' });
  });

  it('strips the heading of a Markdown first line and its marks', () => {
    expect(postTitle('# Что нового в 5.2\n\n**Каналы** — посты.', 'md')).toEqual({ title: 'Что нового в 5.2', rest: '**Каналы** — посты.' });
    expect(postTitle('## Second ##', 'md').title).toBe('Second');
    expect(postTitle('**Bold** and _it_ with [a link](https://x.y) and `code`', 'md').title).toBe('Bold and it with a link and code');
    expect(postTitle('> - [x] snake_case_name', 'md').title).toBe('snake_case_name');
  });

  it('drops the underline of a setext heading', () => {
    expect(postTitle('Title\n=====\nbody', 'md')).toEqual({ title: 'Title', rest: 'body' });
  });

  it('gives no title when the first line of Markdown is a fence, a table or a rule', () => {
    expect(postTitle('```js\nx\n```', 'md')).toEqual({ title: '', rest: '```js\nx\n```' });
    expect(postTitle('| a | b |\n|---|---|', 'md').title).toBe('');
    expect(postTitle('---\ntext', 'md').title).toBe('');
  });

  it('makes a post of one line a title alone', () => {
    expect(postTitle('Сегодня в 22:00 — работы на реле', null)).toEqual({ title: 'Сегодня в 22:00 — работы на реле', rest: '' });
    expect(postTitle('One line\n\n  \n', 'md')).toEqual({ title: 'One line', rest: '' });
  });

  it('has nothing for no words', () => {
    expect(postTitle('', null)).toEqual({ title: '', rest: '' });
    expect(postTitle(undefined, null)).toEqual({ title: '', rest: '' });
    expect(postTitle(' \n\t\n', 'md')).toEqual({ title: '', rest: '' });
  });

  it('skips blank lines before the title and before the text', () => {
    expect(postTitle('\n\n  Title  \n\n\nbody\n  indented\n\n', null)).toEqual({ title: 'Title', rest: 'body\n  indented' });
  });

  it('reads CRLF as lines', () => {
    expect(postTitle('Title\r\nline one\r\nline two', null)).toEqual({ title: 'Title', rest: 'line one\nline two' });
    expect(postTitle('Title\rbody', null)).toEqual({ title: 'Title', rest: 'body' });
  });
});

const text = (t: string) => ({ kind: 'text' as const, text: t });
const BR = { kind: 'break' as const };

describe('dropFirstLine', () => {
  it('drops a heading', () => {
    const blocks: MdBlock[] = [{ kind: 'heading', level: 1, inlines: [text('T')] }, { kind: 'para', inlines: [text('body')] }];
    expect(dropFirstLine(blocks)).toEqual([{ kind: 'para', inlines: [text('body')] }]);
  });

  it('drops the first line of a paragraph, or all of it', () => {
    expect(dropFirstLine([{ kind: 'para', inlines: [text('T'), BR, text('a'), BR, text('b')] }])).toEqual([{ kind: 'para', inlines: [text('a'), BR, text('b')] }]);
    expect(dropFirstLine([{ kind: 'para', inlines: [text('T')] }, { kind: 'rule' }])).toEqual([{ kind: 'rule' }]);
  });

  it('drops the first item of a list, counting on', () => {
    const list: MdBlock = { kind: 'list', ordered: true, start: 1, items: [
      { task: null, blocks: [{ kind: 'para', inlines: [text('one')] }] },
      { task: null, blocks: [{ kind: 'para', inlines: [text('two')] }] },
    ] };
    expect(dropFirstLine([list])).toEqual([{ ...list, start: 2, items: [list.kind === 'list' ? list.items[1] : null] }]);
  });

  it('drops the first line inside a quote', () => {
    const q: MdBlock = { kind: 'quote', blocks: [{ kind: 'para', inlines: [text('T'), BR, text('more')] }] };
    expect(dropFirstLine([q])).toEqual([{ kind: 'quote', blocks: [{ kind: 'para', inlines: [text('more')] }] }]);
    expect(dropFirstLine([{ kind: 'quote', blocks: [{ kind: 'para', inlines: [text('T')] }] }])).toEqual([]);
  });

  it('has nothing of nothing', () => {
    expect(dropFirstLine([])).toEqual([]);
  });
});

describe('editedAt', () => {
  const base = { id: 'a', community_id: 'c', post_id: '', author: 'x', created_at: 1, review: 'ok', hidden: false, deleted: false, pinned: false, reactions: [], comments: 0, pending: false } as FeedItemView;
  it('reads the edit of a core that tells it, none of one that does not', () => {
    expect(editedAt(base)).toBeNull();
    expect(editedAt({ ...base, edited_at: 1700 } as FeedItemView)).toBe(1700);
  });
});

describe('linesOf', () => {
  it('makes one line of the boxes beside each other, top to bottom', () => {
    const lines = linesOf([
      { top: 22, bottom: 44, right: 300 },
      { top: 0, bottom: 22, right: 120 },
      { top: 2, bottom: 20, right: 200 },
      { top: 50, bottom: 72, right: 80 },
    ]);
    expect(lines).toEqual([{ top: 0, bottom: 22, right: 200 }, { top: 22, bottom: 44, right: 300 }, { top: 50, bottom: 72, right: 80 }]);
  });

  it('has none of nothing', () => {
    expect(linesOf([])).toEqual([]);
  });
});

describe('a voice message or a circle as a post', () => {
  const env = (o: Record<string, unknown>) => ({ media_json: JSON.stringify({ v: 1, t: 'media', name: 'voice.weba', mime: 'audio/webm', size: 9, ...o }) });

  it('is read with its kind, its length and its outline', () => {
    const v = mediaOf(env({ kind: 'voice', duration_ms: 3400, waveform: [1, 200, 'x', 255] }));
    expect(v?.kind).toBe('voice');
    expect(v?.duration_ms).toBe(3400);
    expect(v?.waveform).toEqual([1, 200, 255]);
    expect(isRecording(v)).toBe(true);
    const c = mediaOf(env({ kind: 'circle', name: 'circle.webm', duration_ms: 0 }));
    expect([c?.kind, c?.duration_ms, isRecording(c)]).toEqual(['circle', undefined, true]);
    expect(isRecording(mediaOf(env({ kind: 'audio' })))).toBe(false);
    expect(isRecording(null)).toBe(false);
  });

  it('shows its player first with the menu beside it, unless the row above has something to say', () => {
    const card = { comment: false, title: '', deleted: false, recording: true, section: null, pinned: false };
    expect(recordingCard(card)).toBe(true);
    expect(recordingCard({ ...card, section: 'Новости' })).toBe(false);
    expect(recordingCard({ ...card, pinned: true })).toBe(false);
    expect(recordingCard({ ...card, comment: true })).toBe(false);
    expect(recordingCard({ ...card, deleted: true })).toBe(false);
    expect(recordingCard({ ...card, recording: false })).toBe(false);
    expect(recordingCard({ ...card, title: 'Слова' })).toBe(false);
  });
});
