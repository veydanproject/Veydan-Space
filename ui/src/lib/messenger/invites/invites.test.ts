// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the screens of invitations read: a typed code, the kind of an
// address, the person and secret of a link the runtime read, and which
// invitations are live, listed, or shown again instead of drawn anew.

import { describe, expect, it } from 'vitest';
import type { InviteView, LinkView } from '../api';
import {
  addressKind, countdown, endedBy, formatCode, invitedContact, isActive, isPermanentQr, listed, normalizeCode, pausedUntil,
  QR_REFRESH_AHEAD_SECS, reusableQr,
} from './invites';

const NOW = 1_800_000_000;
const inv = (over: Partial<InviteView>): InviteView => ({
  id: 'i', kind: 'link', secret: 's', label: '', created_at: NOW - 100, used: 0, revoked: false, ...over,
});
const contactView = (over: Record<string, unknown> = {}): LinkView => ({
  kind: 'contact', link: 'veydan://contact/npub1x', pubkey: 'ab'.repeat(32), npub: 'npub1x', name: 'Vera', picture: null, nip05: null,
  is_me: false, is_contact: false, blocked: false, ...over,
}) as LinkView;

describe('a typed code', () => {
  it('is six digits, read through spaces and dashes', () => {
    expect(normalizeCode('482913')).toBe('482913');
    expect(normalizeCode(' 482 913 ')).toBe('482913');
    expect(normalizeCode('482-913')).toBe('482913');
    expect(normalizeCode('482 913')).toBe('482913');
  });

  it('is nothing with a letter, too few or too many digits', () => {
    expect(normalizeCode('48291')).toBeNull();
    expect(normalizeCode('4829134')).toBeNull();
    expect(normalizeCode('48291a')).toBeNull();
    expect(normalizeCode('')).toBeNull();
  });

  it('is shown in two groups of three, anything else as it is', () => {
    expect(formatCode('482913')).toBe('482 913');
    expect(formatCode('q7Vd')).toBe('q7Vd');
  });
});

describe('the address of a new chat', () => {
  it('is a key, with or without nostr:, in npub or hex', () => {
    expect(addressKind('npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqq')).toBe('key');
    expect(addressKind('nostr:npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqq')).toBe('key');
    expect(addressKind('ab'.repeat(32))).toBe('key');
  });

  it('is a link of a person only by its scheme and kind, the rest is read by the runtime', () => {
    expect(addressKind('veydan://contact/npub1x?i=q7Vd0xKp3LmN8sRt2WyZaA')).toBe('link');
    expect(addressKind('  veydan://contact/npub1x  ')).toBe('link');
    expect(addressKind('veydan://web/0123456789abcdef')).toBe('link');
    expect(addressKind('veydan://group/abc')).toBeNull();
    expect(addressKind('Vera')).toBeNull();
  });
});

describe('a link the runtime read', () => {
  it('names the person and the secret it carries', () => {
    expect(invitedContact(contactView({ invite: 'q7Vd0xKp3LmN8sRt2WyZaA' }))).toEqual({
      pubkey: 'ab'.repeat(32), name: 'Vera', invite: 'q7Vd0xKp3LmN8sRt2WyZaA', isMe: false, blocked: false, server: null,
    });
  });

  it('carries the card of the server the person is on, when the link has one', () => {
    const server = {
      link: 'veydan://server/e06676e9354ae2e2?u=wss%3A%2F%2Frelay.s1.example&k=&p=off&n=S1', id: 'e06676e9354ae2e2', name: 'S1',
      relay: 'wss://relay.s1.example', media: null, call_node: null, push_url: null, push: 'off', known: false, is_veydan: false,
    };
    expect(invitedContact(contactView({ server }))?.server).toEqual(server);
    expect(invitedContact(contactView())?.server).toBeNull();
  });

  it('without a secret, or from a runtime that does not say one, is a plain link of a person', () => {
    expect(invitedContact(contactView())?.invite).toBeNull();
    expect(invitedContact(contactView({ invite: null }))?.invite).toBeNull();
    expect(invitedContact(contactView({ invite: '' }))?.invite).toBeNull();
  });

  it('is nothing when it leads elsewhere or was not read', () => {
    expect(invitedContact({ kind: 'invalid', code: 'link_bad_param' })).toBeNull();
    expect(invitedContact({ kind: 'unknown', link_type: 'x' })).toBeNull();
    expect(invitedContact(undefined)).toBeNull();
  });

  it('says when it is my own or of someone I blocked', () => {
    expect(invitedContact(contactView({ is_me: true }))?.isMe).toBe(true);
    expect(invitedContact(contactView({ blocked: true }))?.blocked).toBe(true);
  });
});

describe('an invitation', () => {
  it('is live until revoked, past its term or used up', () => {
    expect(isActive(inv({ expires_at: NOW + 1, max_uses: 1 }), NOW)).toBe(true);
    expect(isActive(inv({}), NOW)).toBe(true);
    expect(isActive(inv({ revoked: true }), NOW)).toBe(false);
    expect(isActive(inv({ expires_at: NOW }), NOW)).toBe(false);
    expect(isActive(inv({ max_uses: 1, used: 1 }), NOW)).toBe(false);
    expect(isActive(inv({ used: 40 }), NOW)).toBe(true);
  });

  it('says why it ended, revoked first', () => {
    expect(endedBy(inv({ revoked: true, used: 1, max_uses: 1, expires_at: NOW - 1 }), NOW)).toBe('revoked');
    expect(endedBy(inv({ used: 1, max_uses: 1, expires_at: NOW - 1 }), NOW)).toBe('used');
    expect(endedBy(inv({ expires_at: NOW - 1 }), NOW)).toBe('expired');
    expect(endedBy(inv({ expires_at: NOW + 1 }), NOW)).toBeNull();
  });

  it('is a permanent QR only as a QR without a term', () => {
    expect(isPermanentQr(inv({ kind: 'qr' }))).toBe(true);
    expect(isPermanentQr(inv({ kind: 'qr', expires_at: NOW + 900 }))).toBe(false);
    expect(isPermanentQr(inv({ kind: 'link' }))).toBe(false);
  });
});

describe('the list in the settings', () => {
  it('leaves out the timed QR codes that ended on the screen, live ones first, newest first', () => {
    const list = [
      inv({ id: 'old-link', created_at: NOW - 900, expires_at: NOW - 1 }),
      inv({ id: 'gone-qr', kind: 'qr', created_at: NOW - 2000, expires_at: NOW - 1000 }),
      inv({ id: 'live-qr', kind: 'qr', created_at: NOW - 10, expires_at: NOW + 890 }),
      inv({ id: 'code', kind: 'code', created_at: NOW - 50, expires_at: NOW + 3600, max_uses: 1 }),
      inv({ id: 'revoked-perm', kind: 'qr', created_at: NOW - 5, revoked: true }),
    ];
    expect(listed(list, NOW).map((v) => v.id)).toEqual(['live-qr', 'code', 'revoked-perm', 'old-link']);
  });
});

describe('the QR shown again', () => {
  it('is the newest live one of the kind asked', () => {
    const list = [
      inv({ id: 'a', kind: 'qr', created_at: NOW - 200, expires_at: NOW + 700 }),
      inv({ id: 'b', kind: 'qr', created_at: NOW - 100, expires_at: NOW + 800 }),
      inv({ id: 'p', kind: 'qr', created_at: NOW - 50 }),
      inv({ id: 'l', kind: 'link', created_at: NOW - 10, expires_at: NOW + 9000 }),
    ];
    expect(reusableQr(list, false, NOW)?.id).toBe('b');
    expect(reusableQr(list, true, NOW)?.id).toBe('p');
  });

  it('is none when the timed one is about to end, or the permanent one was revoked', () => {
    const list = [
      inv({ id: 'a', kind: 'qr', expires_at: NOW + QR_REFRESH_AHEAD_SECS }),
      inv({ id: 'p', kind: 'qr', revoked: true }),
    ];
    expect(reusableQr(list, false, NOW)).toBeNull();
    expect(reusableQr(list, true, NOW)).toBeNull();
  });
});

describe('time', () => {
  it('is counted down in minutes and seconds, never below zero', () => {
    expect(countdown(900)).toBe('15:00');
    expect(countdown(65.9)).toBe('1:05');
    expect(countdown(5)).toBe('0:05');
    expect(countdown(-3)).toBe('0:00');
  });

  it('of a pause is over once past', () => {
    expect(pausedUntil(NOW + 60, NOW)).toBe(NOW + 60);
    expect(pausedUntil(NOW, NOW)).toBeNull();
    expect(pausedUntil(null, NOW)).toBeNull();
    expect(pausedUntil(undefined, NOW)).toBeNull();
  });
});
