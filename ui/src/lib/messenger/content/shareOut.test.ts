// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// «Поделиться» outside Veydan: the ways each platform has, the words of my
// contact (the link and nothing else of me), and what each way hands on.

import { beforeEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({ shareOut: vi.fn(async (_via: string, _text: string, _subject?: string) => {}) }));
vi.mock('../api', () => ({ messengerApi: { shareOut: api.shareOut } }));
vi.mock('$lib/core/ui/toast.svelte', () => ({ copied: vi.fn() }));

import { locale } from '$lib/core/i18n';
import { contactShareItem, shareOut, shareOutWays } from './shareOut';

const LINK = 'veydan://contact/npub1abc?i=secret';

describe('shareOut', () => {
  beforeEach(() => {
    api.shareOut.mockClear();
    locale.set('ru');
  });

  it('offers the list of apps and SMS on Android alone', () => {
    expect(shareOutWays('android')).toEqual(['send', 'sms', 'mail', 'copy', 'qr']);
    expect(shareOutWays('linux')).toEqual(['mail', 'copy', 'qr']);
  });

  it('says my contact in one sentence with the link and nothing else', () => {
    const item = contactShareItem(LINK);
    expect(item.text).toBe(`Добавь меня в Veydan Chat\n${LINK}`);
    expect(contactShareItem(LINK, ' Анна ').text).toBe(`Добавь меня в Veydan Chat\n${LINK}\nАнна`);
    expect(item.subject).toBe('Мой контакт в Veydan');
    expect(item.url).toBe(LINK);
    locale.set('en');
    expect(contactShareItem(LINK).text).toBe(`Add me on Veydan Chat\n${LINK}`);
  });

  it('gives a letter its subject, SMS and the list the text alone, the clipboard the link alone', async () => {
    const item = contactShareItem(LINK);
    await shareOut(item, 'mail');
    await shareOut(item, 'sms');
    await shareOut(item, 'send');
    expect(api.shareOut.mock.calls).toEqual([
      ['mail', item.text, item.subject],
      ['sms', item.text, undefined],
      ['send', item.text, undefined],
    ]);
    const writeText = vi.fn(async (_s: string) => {});
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    await shareOut(item, 'copy');
    expect(writeText).toHaveBeenCalledWith(LINK);
    vi.unstubAllGlobals();
  });
});
