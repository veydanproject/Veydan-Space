// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { mockCommand } from '$lib/core/api';
import { IDLE_REQUEST, NO_TRUSTEES, type CloudHelpView, type CloudRequestView, type CloudTrustees } from '$lib/cloud/api';
import {
  AlertsStore,
  daysBetween,
  healthKey,
  helpInputOf,
  HelpStore,
  RecoverStore,
  shortKey,
  socialErrorKey,
  trusteesLine,
  TrusteesStore,
} from '$lib/cloud/social.svelte';

const OWNER = '3f'.repeat(32);
const people = [
  { pubkey: '11'.repeat(32), name: 'Ann' },
  { pubkey: '22'.repeat(32), name: 'Ben' },
  { pubkey: '33'.repeat(32), name: 'Cid' },
  { pubkey: '44'.repeat(32), name: 'Dan' },
];
const configured: CloudTrustees = {
  configured: true, k: 2, n: 3, generation: 1,
  people: people.slice(0, 3).map((p, i) => ({ ...p, index: i + 1, health: i === 2 ? 'waiting' : 'alive', acked_at: 1, checked_at: i === 2 ? null : 1_791_000_000 })),
  acked: 2, alive: 2, checked_at: 1_791_000_000,
};

function refuse(code: string) {
  return () => {
    throw { code, message: code, request_id: null };
  };
}

describe('the words of the screens', () => {
  it('name the health, the days and the errors', () => {
    expect(healthKey('not_delivered')).toBe('cloud_trustee_not_delivered');
    expect(daysBetween(1_791_000_000, 1_791_000_000 + 12 * 86_400 + 5)).toBe(12);
    expect(daysBetween(5, 1)).toBe(0);
    expect(socialErrorKey({ code: 'code_mismatch', message: '', request_id: null })).toBe('cloud_help_err_code_mismatch');
    expect(socialErrorKey({ code: 'frozen', message: '', request_id: null })).toBe('cloud_recover_err_frozen');
    expect(socialErrorKey({ code: 'something_else', message: '', request_id: null })).toBeNull();
    expect(socialErrorKey(null)).toBeNull();
    expect(shortKey(`npub1${'x'.repeat(58)}`)).toBe(`npub1xxxxxxx…xxxx`);
    expect(shortKey('short')).toBe('short');
    expect(trusteesLine(configured, { of: (k, n) => `${k} of ${n}`, alive: (a, n) => `alive ${a}/${n}` })).toBe('2 of 3 · alive 2/3');
    expect(trusteesLine(NO_TRUSTEES, { of: () => 'x', alive: () => 'y' })).toBe('');
  });

  it('read the link of a card into the input of the help', () => {
    expect(helpInputOf(`?npub=${OWNER}&code=AB7K-2QZD-9MXP`)).toBe(`veydan://recover/${OWNER}?c=AB7K-2QZD-9MXP`);
    expect(helpInputOf(`?npub=${OWNER}`)).toBe(OWNER);
    expect(helpInputOf('?code=AB7K')).toBe('');
    expect(helpInputOf('')).toBe('');
  });
});

describe('the trusted people', () => {
  it('picks 3 to 7 people with k within them and sends the shares', async () => {
    let sent: Record<string, unknown> | null = null;
    mockCommand('cloud_trustees_status', NO_TRUSTEES);
    mockCommand('cloud_contacts_approved', people);
    mockCommand('cloud_trustees_setup', (args: Record<string, unknown>) => {
      sent = args;
      return configured;
    });
    const store = new TrusteesStore();
    await store.load();
    store.startFromStatus();
    expect(store.chosen).toEqual([]);
    expect(store.k).toBe(3);
    expect(store.ready).toBe(false);
    for (const p of people.slice(0, 2)) store.toggle(p.pubkey);
    expect(store.k).toBe(2);
    expect(store.ready).toBe(false);
    store.toggle(people[2].pubkey);
    expect(store.ready).toBe(true);
    store.k = 2;
    expect(await store.save()).toBe(true);
    expect(sent).toEqual({ k: 2, people: people.slice(0, 3) });
    expect(store.status.configured).toBe(true);
    // Unticking shrinks k when it was above the count.
    store.k = 3;
    store.toggle(people[2].pubkey);
    expect(store.k).toBe(2);
    // «Заменить» starts the picker without that person.
    store.replace(people[1].pubkey);
    expect(store.chosen).toEqual([people[0].pubkey, people[2].pubkey]);
    expect(store.k).toBe(2);
  });

  it('keeps the error of a refusal and turns the set off', async () => {
    mockCommand('cloud_trustees_status', configured);
    mockCommand('cloud_contacts_approved', people);
    mockCommand('cloud_trustees_setup', refuse('rk_not_issued'));
    const store = new TrusteesStore();
    await store.load();
    store.startFromStatus();
    expect(store.chosen).toHaveLength(3);
    expect(await store.save()).toBe(false);
    expect(socialErrorKey(store.error)).toBe('cloud_social_err_rk_not_issued');
    mockCommand('cloud_trustees_disable', 3);
    mockCommand('cloud_trustees_status', NO_TRUSTEES);
    expect(await store.disable()).toBe(true);
    expect(store.status.configured).toBe(false);
    mockCommand('cloud_trustees_check', configured);
    await store.check();
    expect(store.status.alive).toBe(2);
  });
});

describe('the help to a friend', () => {
  const open: CloudHelpView = {
    npub: OWNER, name: 'Owner', request_id: '9c', code: 'AB7K-2QZD-9MXP', state: 'open',
    created_at: 1, expires_at: 1801, closed_at: null, generation: 1, code_matches: true, can_confirm: true,
  };

  it('looks the request up and confirms only with the checkbox', async () => {
    mockCommand('cloud_help_lookup', open);
    mockCommand('cloud_help_confirm', { request_id: '9c', expires_at: 1801 });
    const store = new HelpStore();
    await store.lookup('');
    expect(store.error?.code).toBe('bad_npub');
    await store.lookup(`veydan://recover/${OWNER}?c=AB7K-2QZD-9MXP`);
    expect(store.phase).toBe('shown');
    expect(store.view?.code).toBe('AB7K-2QZD-9MXP');
    await store.confirmRequest();
    expect(store.phase).toBe('shown');
    expect(store.error?.code).toBe('confirm_required');
    store.spoke = true;
    await store.confirmRequest();
    expect(store.phase).toBe('sent');
  });

  it('shows a tampered or cancelled request without a button', async () => {
    mockCommand('cloud_help_lookup', { ...open, code_matches: false, can_confirm: false });
    const store = new HelpStore();
    await store.lookup(OWNER);
    store.spoke = true;
    await store.confirmRequest();
    expect(store.error?.code).toBe('confirm_required');
    expect(store.phase).toBe('shown');
    mockCommand('cloud_help_lookup', { ...open, state: 'cancelled', closed_at: 900, can_confirm: false });
    await store.lookup(OWNER);
    expect(store.view?.state).toBe('cancelled');
    mockCommand('cloud_help_close', undefined);
    await store.close();
    expect(store.phase).toBe('closed');
    expect(store.view).toBeNull();
    mockCommand('cloud_help_lookup', refuse('no_share'));
    store.reset();
    await store.lookup(OWNER);
    expect(socialErrorKey(store.error)).toBe('cloud_help_err_no_share');
    expect(store.phase).toBe('input');
  });
});

describe('the request of a clean install', () => {
  const waiting: CloudRequestView = {
    ...IDLE_REQUEST, phase: 'waiting', npub: OWNER, request_id: '9c', code: 'AB7K-2QZD-9MXP',
    link: `veydan://recover/${OWNER}?c=AB7K-2QZD-9MXP`, k: 2, n: 3, created_at: 1, expires_at: 1801,
  };

  it('starts, counts the confirmations and finishes with the words', async () => {
    mockCommand('cloud_recover_start', waiting);
    const store = new RecoverStore();
    await store.start('');
    expect(store.error?.code).toBe('bad_npub');
    await store.start(OWNER);
    expect(store.view.phase).toBe('waiting');
    mockCommand('cloud_recover_poll', { ...waiting, confirmed: [{ npub: people[0].pubkey, index: 1, forged: false }] });
    await store.poll();
    expect(store.view.confirmed).toHaveLength(1);
    mockCommand('cloud_recover_poll', { ...waiting, phase: 'collected', confirmed: [{ npub: people[0].pubkey, index: 1, forged: false }, { npub: people[1].pubkey, index: 2, forged: true }] });
    await store.poll();
    expect(store.view.phase).toBe('collected');
    // Nothing more is asked once the key is together.
    mockCommand('cloud_recover_poll', refuse('never'));
    await store.poll();
    expect(store.view.phase).toBe('collected');
    mockCommand('cloud_recover_finish', {
      restored: { npub: OWNER, lock: 'kept', k: 2, n: 3, generation: 1, escrow_refreshed: null },
      words: 'a b c d e f g h i j k l', closed: true,
    });
    expect(await store.finish({ password: '4821', kind: 'pin', hint: null })).toBe(true);
    expect(store.recovered?.words.split(' ')).toHaveLength(12);
    expect(store.view.phase).toBe('restored');
  });

  it('keeps the refusal of the server and cancels', async () => {
    mockCommand('cloud_recover_start', refuse('frozen'));
    const store = new RecoverStore();
    await store.start(OWNER);
    expect(store.view.phase).toBe('failed');
    expect(socialErrorKey(store.error)).toBe('cloud_recover_err_frozen');
    mockCommand('cloud_recover_cancel', undefined);
    await store.cancel();
    expect(store.view).toEqual(IDLE_REQUEST);
    expect(store.error).toBeNull();
  });
});

describe('the alerts of the owner', () => {
  it('loads the alerts, cancels a request and dismisses the rest', async () => {
    const requested = { id: 'c1', t: 'recovery.requested', request_id: '9c', created_at: 1, expires_at: 1801, at: 1 };
    const read = { id: 'c2', t: 'recovery.escrow_read', request_id: null, created_at: null, expires_at: null, at: 2 };
    mockCommand('cloud_recovery_alerts', [requested, read]);
    let cancelledId = '';
    mockCommand('cloud_recovery_cancel', (args: Record<string, unknown>) => {
      cancelledId = String(args.requestId);
      return { state: 'cancelled', frozen_until: 1_791_604_800 };
    });
    let dismissed = '';
    mockCommand('cloud_recovery_alert_dismiss', (args: Record<string, unknown>) => {
      dismissed = String(args.id);
    });
    const store = new AlertsStore();
    await store.load();
    expect(store.alerts).toHaveLength(2);
    expect(await store.cancel('9c')).toBe(true);
    expect(cancelledId).toBe('9c');
    expect(store.cancelled?.frozen_until).toBe(1_791_604_800);
    expect(store.alerts.map((a) => a.id)).toEqual(['c2']);
    await store.dismiss('c2');
    expect(dismissed).toBe('c2');
    expect(store.alerts).toEqual([]);
    mockCommand('cloud_recovery_cancel', refuse('request_closed'));
    expect(await store.cancel('9d')).toBe(false);
    expect(store.error?.code).toBe('request_closed');
  });

  it('remembers which requests are over, for the card in the chat with the bot', async () => {
    const alertOf = (id: string, request: string | null) => ({ id, t: request ? 'recovery.requested' : 'recovery.restored', request_id: request, created_at: 1, expires_at: 1801, at: 1 });
    mockCommand('cloud_recovery_alerts', [alertOf('c1', 'r1'), alertOf('c2', 'r2'), alertOf('c3', 'r3'), alertOf('c4', null)]);
    mockCommand('cloud_recovery_alert_dismiss', undefined);
    const store = new AlertsStore();
    await store.load();
    expect(store.closed).toEqual({});

    // Cancelled here: the answer, with the freeze.
    mockCommand('cloud_recovery_cancel', { state: 'cancelled', frozen_until: 1_791_604_800 });
    await store.cancel('r1');
    expect(store.closed.r1).toEqual({ state: 'cancelled', frozen_until: 1_791_604_800 });

    // Closed by hand: the request is as open as it was.
    await store.dismiss('c2');
    store.take([alertOf('c3', 'r3'), alertOf('c4', null)]);
    expect(store.closed.r2).toBeUndefined();

    // Its alert went without a hand of this device: another device cancelled it.
    store.take([alertOf('c4', null)]);
    expect(store.closed.r3).toEqual({ state: 'cancelled', frozen_until: null });
    expect(store.closed.r1?.frozen_until).toBe(1_791_604_800);

    // Refused as over: nothing is left to cancel. Any other refusal leaves the button.
    mockCommand('cloud_recovery_cancel', refuse('request_closed'));
    await store.cancel('r5');
    expect(store.closed.r5).toEqual({ state: 'closed', frozen_until: null });
    mockCommand('cloud_recovery_cancel', refuse('rate_limited'));
    await store.cancel('r6');
    expect(store.closed.r6).toBeUndefined();
  });
});
