// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { messengerTranslations } from '../i18n';
import { bodyLine, bodyParts, type NoticeBody } from './wording';

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

describe('a line of the list of chats', () => {
  it('puts an icon where a notification puts an emoji, the words the same', () => {
    expect(bodyParts(media('image'), t)).toEqual({ icon: 'image', text: 'Photo' });
    expect(bodyParts(media('video', { duration_ms: 42_000 }), t)).toEqual({ icon: 'video', text: 'Video (0:42)' });
    expect(bodyParts(media('voice', { duration_ms: 12_400 }), t)).toEqual({ icon: 'mic', text: 'Voice message (0:12)' });
    expect(bodyParts(media('circle'), t)).toEqual({ icon: 'circle', text: 'Video message' });
    expect(bodyParts(media('audio', { name: 'song.mp3' }), t)).toEqual({ icon: 'music', text: 'song.mp3' });
    expect(bodyParts(media('file', { name: 'logs.tar.gz', caption: 'by Friday' }), t)).toEqual({ icon: 'paperclip', text: 'logs.tar.gz · by Friday' });
    expect(bodyParts(media('hologram'), t)).toEqual({ icon: 'paperclip', text: 'hologram.bin' });
    expect(bodyParts({ t: 'link', link: 'web', title: 'example.org/a' }, t)).toEqual({ icon: 'link', text: 'example.org/a' });
    expect(bodyParts({ t: 'link', link: 'group', title: '' }, t)).toEqual({ icon: 'link', text: 'Link to a group' });
    expect(bodyParts({ t: 'link', link: 'contact', title: 'Anna' }, t)).toEqual({ icon: 'user', text: 'Contact: Anna' });
  });

  it('has no icon for words alone, and none of the emoji', () => {
    expect(bodyParts({ t: 'text', text: 'hi' }, t)).toEqual({ icon: null, text: 'hi' });
    expect(bodyParts(null, t)).toEqual({ icon: null, text: 'New message' });
    expect(bodyParts({ t: 'invite', group_name: 'Club' }, t).icon).toBeNull();
    for (const kind of ['image', 'video', 'voice', 'circle', 'audio', 'file']) {
      expect(bodyParts(media(kind), t).text).not.toMatch(/\p{Extended_Pictographic}/u);
    }
  });

  it('gives a forward the icon of what it carries', () => {
    expect(bodyParts({ t: 'forwarded', body: media('voice', { duration_ms: 12_400 }) }, t)).toEqual({ icon: 'mic', text: 'Forwarded: Voice message (0:12)' });
    expect(bodyParts({ t: 'forwarded', body: { t: 'text', text: 'hi' } }, t)).toEqual({ icon: null, text: 'Forwarded: hi' });
  });

  it('says what a notification says, less its emoji', () => {
    const bodies: NoticeBody[] = [media('image', { caption: 'the sea' }), media('file', { name: 'a.pdf' }), { t: 'link', link: 'group', title: 'Club' }];
    for (const b of bodies) expect(bodyLine(b, t)).toBe(`${bodyLine(b, t).split(' ')[0]} ${bodyParts(b, t).text}`);
  });
});
