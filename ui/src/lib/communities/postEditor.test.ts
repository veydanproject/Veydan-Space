// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The extended editor of a post: the title and the text joined into the
// words of the post and split again, the draft handed from the composer
// to the editor and back, its marks, and the posts its preview draws.

import { describe, expect, it } from 'vitest';
import {
  applyEditorMark, composerOfDraft, draftOfComposer, editorMarkOfKey, joinPost, PostDrafts, previewId, previewItems, splitPost,
} from './postEditor';
import { postTitle } from './post';
import type { FeedItemView } from './api';

describe('joinPost', () => {
  it('puts the title over the text on a line of its own', () => {
    expect(joinPost('Ранний доступ к 5.4', 'Сборка для тех, кто нас **поддерживает**.', true)).toBe('Ранний доступ к 5.4\nСборка для тех, кто нас **поддерживает**.');
    expect(joinPost('Title', 'one\n\n- two', false)).toBe('Title\none\n\n- two');
  });

  it('is the text alone without a title, the title alone without a text', () => {
    expect(joinPost('', 'Только текст', true)).toBe('Только текст');
    expect(joinPost('  ', '\n\nТекст\n', false)).toBe('Текст');
    expect(joinPost('Заголовок', '  \n ', true)).toBe('Заголовок');
    expect(joinPost('', '', true)).toBe('');
  });

  it('keeps the title one line of plain words in Markdown', () => {
    expect(joinPost('## Заголовок', 'x', true)).toBe('Заголовок\nx');
    expect(joinPost('- not a list', 'x', true)).toBe('\\- not a list\nx');
    expect(joinPost('> not a quote', '', true)).toBe('\\> not a quote');
    expect(joinPost('1. first', '', true)).toBe('\\1. first');
    expect(joinPost('two\nlines', 'x', true)).toBe('two lines\nx');
    // Plain words are sent as written.
    expect(joinPost('# as typed', 'x', false)).toBe('# as typed\nx');
  });

  it('gives the card the title it was written with', () => {
    for (const [title, body] of [['Ранний доступ', 'Текст\n\n- пункт'], ['- минус в начале', 'x'], ['## Заголовок', ''], ['**Важно**', 'текст']]) {
      const text = joinPost(title, body, true);
      expect(postTitle(text, 'md').title).toBe(title.replace(/^#+\s+/, '').replace(/\*\*/g, ''));
    }
  });
});

describe('splitPost', () => {
  it('takes the first line as the title and the rest as the text', () => {
    expect(splitPost('Заголовок\nПервая строка.\n\nВторая.', 'md')).toEqual({ title: 'Заголовок', body: 'Первая строка.\n\nВторая.', md: true });
    expect(splitPost('\n\nОдна строка\n', null)).toEqual({ title: 'Одна строка', body: '', md: false });
    expect(splitPost('', 'md')).toEqual({ title: '', body: '', md: true });
  });

  it('drops a heading of the first line in Markdown, keeps its other marks', () => {
    expect(splitPost('# Что нового\n\n**Каналы**', 'md')).toEqual({ title: 'Что нового', body: '**Каналы**', md: true });
    expect(splitPost('Title\n=====\nbody', 'md').title).toBe('Title');
    expect(splitPost('**Bold** title\nx', 'md').title).toBe('**Bold** title');
    expect(splitPost('# kept as typed\nx', null).title).toBe('# kept as typed');
  });

  it('leaves the title empty under a fence, a table or a rule', () => {
    expect(splitPost('```\ncode\n```', 'md')).toEqual({ title: '', body: '```\ncode\n```', md: true });
    expect(splitPost('---\ntext', 'md').title).toBe('');
  });

  it('is undone by joinPost', () => {
    for (const [title, body, md] of [
      ['Ранний доступ к 5.4', 'Сборка:\n\n- звонки\n- разделы\n\n> Данные сохраняются.', true],
      ['- не список', 'текст', true],
      ['Plain', 'line one\nline two', false],
      ['', 'Только текст', false],
      ['Только заголовок', '', true],
    ] as const) {
      expect(splitPost(joinPost(title, body, md), md ? 'md' : null)).toEqual(
        title ? { title, body, md } : { ...splitPost(body, md ? 'md' : null), md },
      );
    }
  });
});

describe('the draft handed over', () => {
  it('goes from the composer to the editor and back as it was', () => {
    const typed = 'Сбор в субботу\nВстречаемся в **10:00** у входа.\n\n- вода\n- обед';
    const d = draftOfComposer(typed, true);
    expect(d).toEqual({ title: 'Сбор в субботу', body: 'Встречаемся в **10:00** у входа.\n\n- вода\n- обед', md: true });
    expect(composerOfDraft(d)).toEqual({ text: typed, md: true });
    expect(composerOfDraft(draftOfComposer('plain words', false))).toEqual({ text: 'plain words', md: false });
  });

  it('comes back with what the editor changed', () => {
    const d = { ...draftOfComposer('Черновик', true), body: 'дописано в редакторе', md: false };
    expect(composerOfDraft(d)).toEqual({ text: 'Черновик\nдописано в редакторе', md: false });
  });

  it('is kept by chat while the app runs, and taken up by the next ⤢ of an empty composer', () => {
    const drafts = new PostDrafts();
    drafts.set('channel:a', { title: 'Остался', body: 'в редакторе', md: true });
    drafts.set('channel:b', { title: ' ', body: '', md: true });
    expect(drafts.get('channel:b')).toBeNull();
    // The composer has words: they win, the kept draft goes.
    expect(drafts.open('channel:a', 'Новые слова', false)).toEqual({ title: 'Новые слова', body: '', md: false });
    expect(drafts.get('channel:a')).toBeNull();
    drafts.set('channel:a', { title: 'Остался', body: 'в редакторе', md: true });
    // An empty composer: the kept draft comes back, once.
    expect(drafts.open('channel:a', '  ', true)).toEqual({ title: 'Остался', body: 'в редакторе', md: true });
    expect(drafts.open('channel:a', '', true)).toEqual({ title: '', body: '', md: true });
  });
});

describe('the marks of the editor', () => {
  const at = (text: string, mark: Parameters<typeof applyEditorMark>[3], a = 0, b = text.length) => applyEditorMark(text, a, b, mark);

  it('marks the selected words as the composer does', () => {
    expect(at('слово', 'bold')).toEqual({ text: '**слово**', start: 2, end: 7 });
    expect(at('слово', 'italic').text).toBe('*слово*');
    expect(at('a\nb', 'code').text).toBe('```\na\nb\n```');
  });

  it('puts a pair at the caret when nothing is selected', () => {
    expect(at('ab', 'bold', 1, 1)).toEqual({ text: 'a****b', start: 3, end: 3 });
    expect(at('', 'code', 0, 0)).toEqual({ text: '```\n\n```', start: 4, end: 4 });
    expect(at('', 'quote', 0, 0)).toEqual({ text: '> ', start: 2, end: 2 });
  });

  it('makes the lines headings, and takes the same heading off', () => {
    expect(at('Раздел', 'h2', 2, 2)).toEqual({ text: '## Раздел', start: 9, end: 9 });
    expect(at('## Раздел', 'h3', 0, 0).text).toBe('### Раздел');
    expect(at('### Раздел', 'h3', 0, 0).text).toBe('Раздел');
  });

  it('makes lists of the lines, and takes the same list off', () => {
    expect(at('one\ntwo', 'bullets').text).toBe('- one\n- two');
    expect(at('one\n\ntwo', 'numbers').text).toBe('1. one\n\n2. two');
    expect(at('- one\n- two', 'bullets').text).toBe('one\ntwo');
    expect(at('- one\n- two', 'numbers').text).toBe('1. one\n2. two');
  });

  it('makes a link of the words with the address selected, or an empty one at the caret', () => {
    expect(at('сайт', 'link')).toEqual({ text: '[сайт](https://)', start: 7, end: 15 });
    expect(at('', 'link', 0, 0)).toEqual({ text: '[](https://)', start: 1, end: 1 });
  });

  it('puts a rule on a line of its own', () => {
    expect(at('one', 'rule', 3, 3)).toEqual({ text: 'one\n\n---\n\n', start: 10, end: 10 });
    expect(at('', 'rule', 0, 0).text).toBe('---\n\n');
    expect(at('a\n\nb', 'rule', 3, 3).text).toBe('a\n\n---\n\nb');
  });

  it('has the shortcuts of the composer and Ctrl+K for a link', () => {
    const key = (code: string, shiftKey = false) => editorMarkOfKey({ code, ctrlKey: true, metaKey: false, shiftKey, altKey: false });
    expect([key('KeyB'), key('KeyI'), key('KeyK'), key('KeyX', true), key('KeyM', true), key('KeyQ')]).toEqual(['bold', 'italic', 'link', 'strike', 'code', null]);
  });
});

describe('previewItems', () => {
  const base = { communityId: 'c1', author: 'me', at: 1_800_000_000, rich: null };

  it('draws a post of words', () => {
    const [p] = previewItems({ ...base, text: 'Title\nbody', md: false });
    expect(p).toMatchObject({ id: previewId(0), text: 'Title\nbody', fmt: undefined, author: 'me', community_id: 'c1' });
  });

  it('draws the files picked as an album, the words on the last', () => {
    const items = previewItems({ ...base, text: 'Подпись', md: true, rich: [{ kind: 'para', inlines: [{ kind: 'text', text: 'Подпись' }] }], files: [{ key: 1, name: 'a.jpg' }, { key: 2, name: 'b.mp4' }, { key: 3, name: 'setup.apk' }] });
    expect(items.map((x) => x.id)).toEqual([previewId(1), previewId(2), previewId(3)]);
    expect(items.map((x) => JSON.parse(x.media_json ?? '{}').kind)).toEqual(['image', 'video', 'file']);
    expect(items.map((x) => JSON.parse(x.media_json ?? '{}').batch)).toEqual(['preview', 'preview', 'preview']);
    expect(items.slice(0, 2).every((x) => x.text === undefined)).toBe(true);
    expect(items[2]).toMatchObject({ text: 'Подпись', fmt: 'md' });
    expect(items[2].rich).toHaveLength(1);
  });

  it('draws an edit as the post with its new words, its files as they are', () => {
    const post = (id: string, text?: string): FeedItemView => ({
      id, community_id: 'c1', post_id: '', author: 'me', created_at: 1, text, review: 'ok', hidden: false, deleted: false, pinned: false,
      reactions: [], comments: 2, pending: false, media_json: '{"kind":"image","name":"x.jpg"}',
    });
    const parts = [post('p1'), post('p2', 'old')];
    const items = previewItems({ ...base, text: 'new', md: false, edited: parts[1], parts });
    expect(items[0]).toBe(parts[0]);
    expect(items[1]).toMatchObject({ id: 'p2', text: 'new', comments: 2, edited_at: base.at, media_json: parts[1].media_json });
  });
});
