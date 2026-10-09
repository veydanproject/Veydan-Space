// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { bytes as rawBytes, eta as rawEta, fileIcon, rate as rawRate, saveFilters } from './format';

describe('fileIcon', () => {
  it('knows a file by its type, then by its extension', () => {
    for (const [name, mime, icon] of [
      ['a.png', 'image/png', 'image'],
      ['clip.mp4', 'video/mp4', 'video'],
      ['song.mp3', 'audio/mpeg', 'mic'],
      ['tsconfig.json', 'application/octet-stream', 'file-json'],
      ['backup.tar.gz', 'application/gzip', 'archive'],
      ['src.ZIP', '', 'archive'],
      ['report.xlsx', '', 'table-2'],
      ['data.csv', 'text/csv', 'table-2'],
      ['vite.config.js', 'text/javascript', 'file-code'],
      ['dev.sh', 'application/x-sh', 'file-code'],
      ['svelte.config.js', '', 'file-code'],
      ['spec.pdf', 'application/pdf', 'file-text'],
      ['notes.txt', 'text/plain', 'file-text'],
      ['letter', 'application/msword', 'file-text'],
      ['firmware.bin', 'application/octet-stream', 'file'],
      ['README', '', 'file'],
    ] as const) {
      expect(fileIcon(name, mime), name).toBe(icon);
    }
  });
});

// The space between a number and its unit does not break; written here as a plain one.
const plain = (s: string) => s.replace(/\u00a0/g, ' ');
const bytes = (n: number, lang?: string) => plain(rawBytes(n, lang));
const rate = (n: number, lang?: string) => plain(rawRate(n, lang));
const eta = (n: number | null, lang?: string) => plain(rawEta(n, lang));

describe('bytes, rate and eta', () => {
  it('keeps the plain English form without a language', () => {
    expect(bytes(0)).toBe('0 B');
    expect(bytes(512)).toBe('512 B');
    expect(bytes(1.4 * 1024 * 1024)).toBe('1.4 MB');
    expect(bytes(330 * 1024 * 1024)).toBe('330 MB');
  });

  it('writes the number and the unit in the language asked', () => {
    expect(bytes(5.2 * 1024 * 1024, 'ru')).toBe('5,2 МБ');
    expect(bytes(812 * 1024 * 1024, 'ru')).toBe('812 МБ');
    expect(bytes(1024 ** 3, 'ru')).toBe('1,0 ГБ');
    expect(bytes(5.2 * 1024 * 1024, 'en')).toBe('5.2 MB');
  });

  it('never shows 1024 of a unit: just under it is 1.0 of the next one', () => {
    expect(bytes(1023.7 * 1024 * 1024, 'ru')).toBe('1,0 ГБ');
    expect(bytes(1023.6 * 1024 * 1024, 'en')).toBe('1.0 GB');
    expect(bytes(1023.7 * 1024, 'ru')).toBe('1,0 МБ');
    expect(bytes(1023.7 * 1024)).toBe('1.0 MB');
    expect(bytes(1023.4 * 1024)).toBe('1023 KB');
  });

  it('gives a speed per second, nothing while nothing moves', () => {
    expect(rate(5.2 * 1024 * 1024, 'ru')).toBe('5,2 МБ/с');
    expect(rate(5.2 * 1024 * 1024, 'en')).toBe('5.2 MB/s');
    // The language's own short second may be a word: never `MB/Sek.`.
    expect(rate(5.2 * 1024 * 1024, 'de')).toBe('5,2 MB/s');
    expect(rate(512 * 1024, 'fr')).toBe('512 ko/s');
    expect(rate(0, 'ru')).toBe('');
  });

  it('rounds the time left to what a person reads', () => {
    expect(eta(40, 'ru')).toBe('~40 с');
    expect(eta(3, 'ru')).toBe('~5 с');
    expect(eta(58, 'ru')).toBe('~1 мин');
    expect(eta(100, 'ru')).toBe('~2 мин');
    expect(eta(3700, 'ru')).toBe('~1 ч');
    expect(eta(130, 'en')).toBe('~2 min');
    expect(eta(null, 'ru')).toBe('');
    expect(eta(0, 'ru')).toBe('');
  });
});

describe('saveFilters', () => {
  it('names the extension of the file, then every file', () => {
    expect(saveFilters('Holiday.JPG', 'All files')).toEqual([
      { name: 'JPG', extensions: ['jpg'] },
      { name: 'All files', extensions: ['*'] },
    ]);
    expect(saveFilters('backup.tar.gz', 'All')[0]).toEqual({ name: 'GZ', extensions: ['gz'] });
  });

  it('has none for a name without an extension, or one that is no extension', () => {
    for (const name of ['README', '', 'photo.', 'a.b c', 'x.' + 'e'.repeat(13)]) expect(saveFilters(name, 'All files'), name).toEqual([]);
  });
});
