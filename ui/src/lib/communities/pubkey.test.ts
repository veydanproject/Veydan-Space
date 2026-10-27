// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { looksLikeKey, npubToHex, personKey } from './pubkey';

// The example of NIP-19.
const HEX = '7e7e9c42a91bfef19fa929e5fda1b72e0ebc1a4c1141673e2794234d86addf4e';
const NPUB = 'npub10elfcs4fr0l0r8af98jlmgdh9c8tcxjvz9qkw038js35mp4dma8qzvjptg';

describe('npubToHex', () => {
  it('decodes an npub, in either case', () => {
    expect(npubToHex(NPUB)).toBe(HEX);
    expect(npubToHex(NPUB.toUpperCase())).toBe(HEX);
    expect(npubToHex('npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqzqujme')).toBe('00'.repeat(32));
    expect(npubToHex('npub1lllllllllllllllllllllllllllllllllllllllllllllllllllsq7lrjw')).toBe('ff'.repeat(32));
  });

  it('refuses a wrong checksum, a mixed case, another prefix, a wrong length', () => {
    expect(npubToHex(NPUB.slice(0, -1) + 'h')).toBeNull();
    expect(npubToHex(NPUB.replace('0elf', '0elg'))).toBeNull();
    expect(npubToHex('npub1' + NPUB.slice(5, 10).toUpperCase() + NPUB.slice(10))).toBeNull();
    expect(npubToHex('nsec' + NPUB.slice(4))).toBeNull();
    expect(npubToHex('npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq')).toBeNull();
    expect(npubToHex('npub1')).toBeNull();
    expect(npubToHex('')).toBeNull();
  });
});

describe('personKey', () => {
  it('gives the hex the commands take, for an npub and for hex', () => {
    expect(personKey(NPUB)).toBe(HEX);
    expect(personKey(`  ${NPUB}\n`)).toBe(HEX);
    expect(personKey(HEX)).toBe(HEX);
    expect(personKey(HEX.toUpperCase())).toBe(HEX);
  });

  it('is null for anything else', () => {
    expect(personKey(HEX.slice(1))).toBeNull();
    expect(personKey(HEX + '0')).toBeNull();
    expect(personKey('Алина')).toBeNull();
    expect(personKey(NPUB.slice(0, -2))).toBeNull();
  });
});

describe('looksLikeKey', () => {
  it('reads an npub (valid or not) and 64 hex as a key, a name as a search', () => {
    expect(looksLikeKey(NPUB)).toBe(true);
    expect(looksLikeKey(NPUB.slice(0, -1) + 'h')).toBe(true);
    expect(looksLikeKey(HEX)).toBe(true);
    expect(looksLikeKey('npub')).toBe(false);
    expect(looksLikeKey('Алина')).toBe(false);
    expect(looksLikeKey(HEX.slice(1))).toBe(false);
  });
});
