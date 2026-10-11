// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { mockCommand } from '$lib/core/api';
import {
  linkErrorKey,
  NewDeviceLink,
  OldDeviceLink,
  qrDataUrl,
  timeLeft,
  type LinkDone,
  type LinkStart,
} from '$lib/cloud/link.svelte';

const T = 'ab'.repeat(32);
const shown: LinkStart = {
  t_pub: T,
  words: ['abandon', 'ability', 'able', 'about', 'above', 'absent', 'absorb', 'abstract'],
  uri: `veydan://link/${T}/abandon-ability-able-about-above-absent-absorb-abstract`,
  qr_svg: '<svg/>',
  fingerprint: ['oven', 'humor', 'race', 'kiwi'],
  expires_at: 1_791_000_600,
};
const done: LinkDone = { npub: '3f'.repeat(32), done_sent: true, vault_joined: false };

function refuse(code: string) {
  return () => {
    throw { code, message: code, request_id: null };
  };
}

describe('the new device', () => {
  it('shows the code, waits, and takes the account over when it comes', async () => {
    mockCommand('cloud_link_start', shown);
    mockCommand('cloud_link_poll', { state: 'waiting', expires_at: shown.expires_at, npub: null });
    const link = new NewDeviceLink();
    await link.begin();
    expect(link.phase).toBe('waiting');
    expect(link.shown?.words).toHaveLength(8);
    await link.poll();
    expect(link.phase).toBe('waiting');

    mockCommand('cloud_link_poll', { state: 'received', expires_at: shown.expires_at, npub: done.npub });
    mockCommand('cloud_link_receive', done);
    await link.poll();
    expect(link.phase).toBe('done');
    expect(link.done).toEqual(done);
    expect(link.shown).toBeNull();
    // Nothing more to look at once it is done.
    await link.poll();
    expect(link.phase).toBe('done');
  });

  it('asks for the password of a closed lock and keeps the link between the tries', async () => {
    mockCommand('cloud_link_start', shown);
    mockCommand('cloud_link_poll', { state: 'received', expires_at: shown.expires_at, npub: done.npub });
    mockCommand('cloud_link_receive', refuse('locked'));
    const link = new NewDeviceLink();
    await link.begin();
    await link.poll();
    expect(link.phase).toBe('received');
    expect(link.needsPassword).toBe(true);
    expect(link.error).toBeNull();

    mockCommand('cloud_link_receive', refuse('wrong_password'));
    await link.receive('0000');
    expect(link.phase).toBe('received');
    expect(linkErrorKey(link.error)).toBe('cloud_link_err_wrong_password');

    mockCommand('cloud_link_receive', done);
    await link.receive('4821');
    expect(link.phase).toBe('done');
    expect(link.needsPassword).toBe(false);
  });

  it('offers to go on without a storage that is out of reach, and keeps the link', async () => {
    mockCommand('cloud_link_start', shown);
    mockCommand('cloud_link_poll', { state: 'received', expires_at: shown.expires_at, npub: done.npub });
    for (const [code, key] of [
      ['vault_unreachable', 'cloud_vault_unreachable'],
      ['not_clean', 'cloud_restore_not_clean'],
    ] as const) {
      mockCommand('cloud_link_receive', refuse(code));
      const link = new NewDeviceLink();
      await link.begin();
      await link.poll();
      expect(link.phase).toBe('received');
      expect(link.vaultAway).toBe(true);
      expect(link.needsPassword).toBe(false);
      expect(linkErrorKey(link.error)).toBe(key);

      mockCommand('cloud_link_receive', { ...done, vault_joined: false });
      await link.receive(null, true);
      expect(link.phase).toBe('done');
      expect(link.vaultAway).toBe(false);
      expect(link.done?.vault_joined).toBe(false);
    }
  });

  it('says the code is out of date, and a new start is a new code', async () => {
    mockCommand('cloud_link_start', shown);
    mockCommand('cloud_link_poll', { state: 'expired', expires_at: shown.expires_at, npub: null });
    const link = new NewDeviceLink();
    await link.begin();
    await link.poll();
    expect(link.phase).toBe('expired');
    mockCommand('cloud_link_start', { ...shown, t_pub: 'cd'.repeat(32) });
    await link.begin();
    expect(link.phase).toBe('waiting');
    expect(link.shown?.t_pub).toBe('cd'.repeat(32));
  });

  it('publishes nothing without the relays and says so', async () => {
    mockCommand('cloud_link_start', refuse('no_relay'));
    const link = new NewDeviceLink();
    await link.begin();
    expect(link.phase).toBe('failed');
    expect(link.shown).toBeNull();
    expect(linkErrorKey(link.error)).toBe('cloud_link_err_no_relay');
    mockCommand('cloud_link_cancel', null);
    await link.cancel();
    expect(link.phase).toBe('idle');
  });
});

describe('the old device', () => {
  it('finds the new device, compares, and sends after the password', async () => {
    const target = { t_pub: T, fingerprint: shown.fingerprint, expires_at: shown.expires_at };
    mockCommand('cloud_link_resolve', target);
    mockCommand('cloud_link_approve', { t_pub: T, expires_at: shown.expires_at });
    const link = new OldDeviceLink();
    await link.resolve(shown.words.join(' '));
    expect(link.phase).toBe('compare');
    expect(link.target?.fingerprint).toEqual(shown.fingerprint);
    await link.approve('4821');
    expect(link.phase).toBe('sent');
  });

  it('keeps the comparison when the password is wrong, and sends nothing on a mismatch', async () => {
    mockCommand('cloud_link_resolve', { t_pub: T, fingerprint: shown.fingerprint, expires_at: shown.expires_at });
    mockCommand('cloud_link_approve', refuse('wrong_password'));
    mockCommand('cloud_link_cancel', null);
    const link = new OldDeviceLink();
    await link.resolve(shown.uri);
    await link.approve('0000');
    expect(link.phase).toBe('compare');
    expect(linkErrorKey(link.error)).toBe('cloud_link_err_wrong_password');
    await link.mismatch();
    expect(link.phase).toBe('mismatch');
    expect(link.target).toBeNull();
    await link.reset();
    expect(link.phase).toBe('input');
  });

  it('keeps the wrap on the relays when it goes back after sending', async () => {
    let cancels = 0;
    mockCommand('cloud_link_resolve', { t_pub: T, fingerprint: shown.fingerprint, expires_at: shown.expires_at });
    mockCommand('cloud_link_approve', { t_pub: T, expires_at: shown.expires_at });
    mockCommand('cloud_link_cancel', () => {
      cancels += 1;
      return null;
    });
    const link = new OldDeviceLink();
    await link.resolve(shown.uri);
    await link.approve('4821');
    expect(link.phase).toBe('sent');
    await link.reset();
    expect(link.phase).toBe('input');
    expect(link.target).toBeNull();
    expect(cancels).toBe(0);
    // Before sending, «Назад» ends the link.
    await link.resolve(shown.uri);
    await link.reset();
    expect(cancels).toBe(1);
  });

  it('says what is wrong with what was typed', async () => {
    const link = new OldDeviceLink();
    await link.resolve('   ');
    expect(link.phase).toBe('input');
    expect(linkErrorKey(link.error)).toBe('cloud_link_err_code');
    for (const [code, key] of [
      ['link_not_found', 'cloud_link_err_not_found'],
      ['link_code', 'cloud_link_err_code'],
      ['no_relay', 'cloud_link_err_no_relay'],
      ['no_host', 'cloud_link_err_own_servers'],
    ]) {
      mockCommand('cloud_link_resolve', refuse(code));
      await link.resolve('abandon about');
      expect(link.phase).toBe('input');
      expect(linkErrorKey(link.error)).toBe(key);
    }
  });
});

describe('the helpers of a link', () => {
  it('counts the time left and draws the QR code as an image', () => {
    expect(timeLeft(1000, 400)).toBe('10:00');
    expect(timeLeft(1000, 991)).toBe('0:09');
    expect(timeLeft(1000, 2000)).toBe('0:00');
    expect(qrDataUrl('<svg/>')).toBe(`data:image/svg+xml;base64,${btoa('<svg/>')}`);
    expect(linkErrorKey(null)).toBeNull();
    expect(linkErrorKey({ code: 'something_new', message: 'x', request_id: null })).toBeNull();
  });
});
