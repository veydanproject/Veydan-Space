// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { Proxy } from '$lib/browser/types';
import { proxyAddress, proxyChip, proxyOptionLabel, proxyTypeName, torCountries, torLabel } from './proxy-label';

const tr = (key: string) => (key === 'proxy_tor_any' ? 'any country' : key);

const row = (over: Partial<Proxy>) =>
  ({ name: 'Row', proxy_type: 'socks5', host: '1.2.3.4', port: 1080, country: null, city: null, ...over }) as Proxy;

describe('the labels of a Tor proxy', () => {
  it('shows the exit countries in capitals', () => {
    expect(torCountries('de,nl')).toBe('DE, NL');
    expect(torCountries(' de , NL ')).toBe('DE, NL');
    expect(torCountries(null)).toBe('');
    expect(torCountries('')).toBe('');
  });

  it('says any country when none is set', () => {
    expect(torLabel('de,nl', tr)).toBe('Tor · DE, NL');
    expect(torLabel(null, tr)).toBe('Tor · any country');
    expect(torLabel('', tr)).toBe('Tor · any country');
  });

  it('never shows the placeholder address of a Tor row', () => {
    const tor = row({ proxy_type: 'tor', host: '127.0.0.1', port: 9050, country: 'de' });
    expect(proxyAddress(tor, tr)).toBe('Tor · DE');
    expect(proxyOptionLabel(tor, tr)).toBe('Row (Tor · DE)');
    expect(proxyOptionLabel(row({ proxy_type: 'tor', host: '127.0.0.1', port: 9050 }), tr)).toBe('Row (Tor · any country)');
  });

  it('keeps the labels of the other proxies', () => {
    expect(proxyAddress(row({}), tr)).toBe('1.2.3.4:1080');
    expect(proxyOptionLabel(row({}), tr)).toBe('Row');
    expect(proxyOptionLabel(row({ country: 'RU', city: 'Moscow' }), tr)).toBe('Row (RU, Moscow)');
  });
});

describe('the chips of a proxy', () => {
  it('shows the exit countries of Tor and Tor itself for any country', () => {
    expect(proxyChip(row({ proxy_type: 'tor', country: 'de,nl' }))).toBe('DE, NL');
    expect(proxyChip(row({ proxy_type: 'tor' }))).toBe('Tor');
    expect(proxyChip(row({ country: 'RU' }))).toBe('RU');
    expect(proxyChip(row({}))).toBeNull();
    expect(proxyTypeName(row({ proxy_type: 'tor' }))).toBe('Tor');
    expect(proxyTypeName(row({}))).toBe('socks5');
  });
});
