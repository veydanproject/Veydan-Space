// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { messengerTranslations } from '../i18n';
import { albumLine, albumOf, bodyLine, duration, joinAlbum, replyLine, replyThumb, type NoticeBody } from './wording';

const en = messengerTranslations.en as Record<string, string>;
const t = (key: string, vars?: Record<string, string>) => {
  let text = en[key] ?? key;
  for (const [k, v] of Object.entries(vars ?? {})) text = text.split(`{${k}}`).join(v);
  return text;
};

const media = (kind: string, extra: Partial<Extract<NoticeBody, { t: 'media' }>> = {}): NoticeBody => ({
  t: 'media',
  kind,
  name: `${kind}.bin`,
  ...extra,
});

describe('a notification line', () => {
  it('tells a file by what it is, never by a missing caption', () => {
    expect(bodyLine(media('image'), t)).toBe('📷 Photo');
    expect(bodyLine(media('image', { caption: 'the sea' }), t)).toBe('📷 the sea');
    expect(bodyLine(media('video', { duration_ms: 42_000 }), t)).toBe('🎬 Video (0:42)');
    expect(bodyLine(media('voice', { duration_ms: 12_400 }), t)).toBe('🎤 Voice message (0:12)');
    expect(bodyLine(media('file', { name: 'report.pdf' }), t)).toBe('📎 report.pdf');
    expect(bodyLine(media('file', { name: 'report.pdf', caption: 'by Friday' }), t)).toBe('📎 report.pdf · by Friday');
    expect(bodyLine(media('hologram'), t)).toBe('📎 hologram.bin');
    expect(bodyLine(null, t)).toBe('New message');
  });

  it('tells a link by what it names', () => {
    expect(bodyLine({ t: 'link', link: 'web', title: 'example.org/a' }, t)).toBe('🔗 example.org/a');
    expect(bodyLine({ t: 'link', link: 'group', title: 'Club' }, t)).toBe('🔗 Group “Club”');
    expect(bodyLine({ t: 'link', link: 'group', title: '' }, t)).toBe('🔗 Link to a group');
    expect(bodyLine({ t: 'link', link: 'contact', title: 'Anna' }, t)).toBe('👤 Contact: Anna');
  });

  it('counts an album instead of naming its pictures', () => {
    const first = albumOf(media('image', { batch: 'b1' }));
    const second = albumOf(media('video', { batch: 'b1', caption: 'our trip' }));
    const joined = joinAlbum(first, second);
    expect(joined && albumLine(joined, t)).toBe('🖼 Album: 2 · our trip');
    expect(joinAlbum(joined, albumOf(media('image', { batch: 'b2' })))).toBeNull();
    const files = joinAlbum(albumOf(media('file', { batch: 'f' })), albumOf(media('file', { batch: 'f' })));
    expect(files && albumLine(files, t)).toBe('📎 Files: 2');
    expect(albumOf(media('voice', { batch: 'b1' }))).toBeNull();
    expect(albumOf(media('image'))).toBeNull();
  });

  it('writes a length as a clock does', () => {
    expect(duration(0)).toBe('0:00');
    expect(duration(59_600)).toBe('1:00');
    expect(duration(3_723_000)).toBe('1:02:03');
  });
});

describe('the picture of a quote', () => {
  it('shows the JPEG preview of a photo or a video, nothing else', () => {
    const thumb = '/9j/4AECAw==';
    expect(replyThumb({ text: null, media: { kind: 'image', thumb } })).toBe(`data:image/jpeg;base64,${thumb}`);
    expect(replyThumb({ text: 'look', media: { kind: 'video', thumb } })).toBe(`data:image/jpeg;base64,${thumb}`);
    expect(replyThumb({ text: null, deleted: true, media: { kind: 'image', thumb } })).toBeNull();
    expect(replyThumb({ text: null, media: { kind: 'file', thumb } })).toBeNull();
    expect(replyThumb({ text: null, media: { kind: 'image' } })).toBeNull();
    expect(replyThumb({ text: null, media: { kind: 'image', thumb: 'iVBORw0KGgo=' } })).toBeNull();
    expect(replyThumb({ text: 'hi' })).toBeNull();
  });
});

describe('the line under a reply', () => {
  it('tells a message taken back first, then the text, a card, a file', () => {
    expect(replyLine({ text: null, deleted: true }, null, t)).toBe('Message deleted');
    expect(replyLine({ text: 'hello' }, null, t)).toBe('hello');
    expect(replyLine({ text: null }, 'Carol', t)).toBe('👤 Carol');
    expect(replyLine({ text: null, media: { kind: 'image', name: 'cat.png' } }, null, t)).toBe('📷 Photo');
    expect(replyLine({ text: null, media: { kind: 'voice', name: 'v.webm', duration_ms: 12_400 } }, null, t)).toBe('🎤 Voice message (0:12)');
    expect(replyLine({ text: null, media: { kind: 'file', name: 'report.pdf' } }, null, t)).toBe('📎 report.pdf');
  });

  it('never calls a photo without a caption deleted, and leaves the unknown empty', () => {
    expect(replyLine({ text: null, deleted: false, media: { kind: 'video' } }, null, t)).not.toBe('Message deleted');
    expect(replyLine({ text: null }, null, t)).toBe('');
    expect(replyLine({ text: null, media: {} }, null, t)).toBe('');
  });

  it('an old runtime that sends no flag shows the text it has', () => {
    expect(replyLine({ text: 'hi', deleted: undefined }, null, t)).toBe('hi');
  });
});
