// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// npubToHex copied from sdk/integrations/openclaw/src/targets.ts at 11857373.

// The key of a person as the page takes it (npub1… or 64 hex) and as the
// commands of the module take it (64 hex, lower case): the core parses hex
// alone (crates/communities `key`), so an npub is decoded here.

const BECH32_ALPHABET = 'qpzry9x8gf2tvdw0s3jn54khce6mua7l';
const HEX64 = /^[0-9a-f]{64}$/i;

function polymod(values: number[]): number {
  const gen = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
  let chk = 1;
  for (const v of values) {
    const top = chk >>> 25;
    chk = ((chk & 0x1ffffff) << 5) ^ v;
    for (let i = 0; i < 5; i++) if ((top >>> i) & 1) chk ^= gen[i]!;
  }
  return chk >>> 0;
}

function hrpExpand(hrp: string): number[] {
  const out: number[] = [];
  for (const c of hrp) out.push(c.charCodeAt(0) >> 5);
  out.push(0);
  for (const c of hrp) out.push(c.charCodeAt(0) & 31);
  return out;
}

function convertBits(data: number[], from: number, to: number): number[] | null {
  let acc = 0;
  let bits = 0;
  const out: number[] = [];
  const max = (1 << to) - 1;
  for (const value of data) {
    acc = (acc << from) | value;
    bits += from;
    while (bits >= to) {
      bits -= to;
      out.push((acc >> bits) & max);
    }
  }
  if (bits >= from || ((acc << (to - bits)) & max) !== 0) return null;
  return out;
}

/** The 32 bytes of an npub as hex, or null when it is not a valid npub (a wrong checksum among). */
export function npubToHex(npub: string): string | null {
  const s = npub.trim();
  if (s !== s.toLowerCase() && s !== s.toUpperCase()) return null;
  const lower = s.toLowerCase();
  const sep = lower.lastIndexOf('1');
  if (sep < 1 || lower.slice(0, sep) !== 'npub' || lower.length - sep - 1 < 6) return null;
  const data: number[] = [];
  for (const c of lower.slice(sep + 1)) {
    const v = BECH32_ALPHABET.indexOf(c);
    if (v < 0) return null;
    data.push(v);
  }
  if (polymod([...hrpExpand('npub'), ...data]) !== 1) return null;
  const bytes = convertBits(data.slice(0, -6), 5, 8);
  if (!bytes || bytes.length !== 32) return null;
  return bytes.map((b) => b.toString(16).padStart(2, '0')).join('');
}

/** A typed key as 64 lower-case hex; null when it is neither a valid npub nor 64 hex. */
export function personKey(raw: string): string | null {
  const s = raw.trim();
  if (HEX64.test(s)) return s.toLowerCase();
  return npubToHex(s);
}

/** What the field holds reads as a key to try (not a name of a contact being searched). */
export const looksLikeKey = (raw: string): boolean => /^(npub1[0-9a-z]+|[0-9a-f]{64})$/i.test(raw.trim());
