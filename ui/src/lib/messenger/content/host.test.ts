// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { canPreview, describeExternal } from './host';
import { tokenize } from './tokenize';
import type { ExternalUrl } from './types';

function url(text: string): ExternalUrl {
  const s = tokenize(text)[0];
  if (s?.type !== 'external') throw new Error(`not a link outside: ${text}`);
  return s.url;
}

describe('describeExternal', () => {
  it('shows where a link leads', () => {
    expect(describeExternal(url('https://Example.com/News/1?x=1#top'))).toEqual({ host: 'example.com', path: '/News/1?x=1', secure: true, risks: [] });
    expect(describeExternal(url('https://example.com/'))).toEqual({ host: 'example.com', path: '', secure: true, risks: [] });
    expect(describeExternal(url('https://example.com/%D0%BF%D1%83%D1%82%D1%8C')).path).toBe('/путь');
    const long = describeExternal(url(`https://example.com/${'a'.repeat(200)}`)).path;
    expect([...long].length).toBe(60);
    expect(long.endsWith('…')).toBe(true);
  });

  it('says what deserves a second look', () => {
    expect(describeExternal(url('http://example.com')).risks).toEqual(['insecure']);
    expect(describeExternal(url('https://аррӏе.com/')).risks).toEqual(['lookalike']);
    expect(describeExternal(url('https://xn--80ak6aa92e.com/')).host).toBe('xn--80ak6aa92e.com');
    expect(describeExternal(url('https://93.184.216.34/a')).risks).toEqual(['address']);
    expect(describeExternal(url('https://[2606:2800:220:1::1]/a')).risks).toEqual(['address']);
    expect(describeExternal(url('https://bank.example@evil.example/')).risks).toEqual(['userinfo']);
    expect(describeExternal(url('https://bank.example@evil.example/')).host).toBe('evil.example');
    expect(describeExternal(url('http://example.com:8080/')).risks).toEqual(['insecure', 'port']);
    expect(describeExternal(url('https://example.com:443/')).risks).toEqual([]);
  });

  it('asks for a preview only what may be asked', () => {
    expect(canPreview(url('https://example.com/a'))).toBe(true);
    expect(canPreview(url('http://example.com/a'))).toBe(false);
    expect(canPreview(url('https://192.168.1.1/'))).toBe(false);
    expect(canPreview(url('https://a@example.com/'))).toBe(false);
  });
});
