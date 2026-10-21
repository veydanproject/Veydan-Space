// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { CloudAccount, CloudStatus } from '$lib/cloud/api';
import {
  deleteReady,
  emptyDeleteForm,
  errorKeyOf,
  dateOf,
  formatBytes,
  normalizeKey,
  poolsOf,
  SHARES_REVOCABLE,
  shownAccount,
  totalPool,
  viewOf,
} from '$lib/cloud/account';

const account: CloudAccount = {
  npub: '3f'.repeat(32),
  state: 'paid',
  plan: 'plus',
  until: 1793604345,
  grace_until: null,
  limits: { 'vault.files_bytes': 2 * 1024 ** 3, 'vault.text_bytes': 512 * 1024 ** 2, 'media.retention_days': 90 },
  usage: { as_of: 1, vault: { files_bytes: 1024 ** 3, text_bytes: 0 }, media: { upload_today_bytes: 0, storage_bytes: 7340032 } },
  recovery: { configured: true, k: 3, n: 5, generation: 2 },
};

const status = (s: Partial<CloudStatus>): CloudStatus => ({ configured: false, account: null, plan: null, ...s });

describe('the screen of the page', () => {
  it('follows whose servers and what the server said', () => {
    expect(viewOf(null)).toBe('loading');
    expect(viewOf(status({}))).toBe('unavailable');
    expect(viewOf(status({ servers: 'own' }))).toBe('own');
    expect(viewOf(status({ servers: 'unchosen' }))).toBe('unchosen');
    expect(viewOf(status({ servers: 'veydan' }))).toBe('unreachable');
    expect(viewOf(status({ servers: 'veydan', configured: true }))).toBe('no_account');
    expect(viewOf(status({ servers: 'veydan', configured: true, account }))).toBe('account');
    // The status of C0, without `servers`.
    expect(viewOf(status({ configured: true, account }))).toBe('account');
  });

  it('shows the live account, or the one kept when the server is away', () => {
    expect(shownAccount(null)).toBeNull();
    const live = shownAccount(status({ account }));
    expect(live?.plan).toBe('plus');
    expect(live?.keptAt).toBeNull();
    expect(live?.recovery?.k).toBe(3);
    const kept = shownAccount(
      status({
        servers: 'veydan',
        kept: {
          npub: account.npub,
          state: 'free',
          plan: 'free',
          until: null,
          grace_until: null,
          limits: {},
          usage: null,
          last_seen_at: null,
          refreshed_at: 1791012345,
        },
      }),
    );
    expect(kept?.plan).toBe('free');
    expect(kept?.keptAt).toBe(1791012345);
  });
});

describe('the pools of the account', () => {
  it('weigh the vault against the plan and leave the media of chats without a limit', () => {
    const pools = poolsOf(shownAccount(status({ account })));
    expect(pools.map((p) => p.id)).toEqual(['files', 'text', 'media']);
    expect(pools[0]).toEqual({ id: 'files', used: 1024 ** 3, limit: 2 * 1024 ** 3, percent: 50 });
    expect(pools[1].percent).toBe(0);
    expect(pools[2]).toEqual({ id: 'media', used: 7340032, limit: null, percent: null });
    expect(poolsOf(null)).toEqual([]);
    expect(totalPool(pools)).toBeNull();
  });

  it('put the shared storage of the plan first, counting files, text and media together', () => {
    const gib = 1024 ** 3;
    const shared: CloudAccount = {
      ...account,
      limits: { 'storage.total_bytes': 10 * gib, 'vault.files_bytes': 10 * gib, 'vault.text_bytes': 10 * gib },
      usage: { as_of: 1, vault: { files_bytes: 2 * gib, text_bytes: gib / 2 }, media: { upload_today_bytes: 0, storage_bytes: gib / 2 } },
    };
    const pools = poolsOf(shownAccount(status({ account: shared })));
    expect(pools.map((p) => p.id)).toEqual(['total', 'files', 'text', 'media']);
    expect(pools[0]).toEqual({ id: 'total', used: 3 * gib, limit: 10 * gib, percent: 30 });
    expect(totalPool(pools)).toBe(pools[0]);
    // Nothing of the vault or the media yet: the total is there, empty.
    const fresh = poolsOf(shownAccount(status({ account: { ...shared, usage: null } })));
    expect(fresh[0]).toEqual({ id: 'total', used: 0, limit: 10 * gib, percent: 0 });
    // A total of 0 is no shared storage.
    const none = poolsOf(shownAccount(status({ account: { ...shared, limits: { 'storage.total_bytes': 0 } } })));
    expect(none.map((p) => p.id)).toEqual(['files', 'text', 'media']);
  });

  it('write bytes for a person', () => {
    expect(formatBytes(0)).toBe('0 B');
    expect(formatBytes(1536)).toBe('1.5 KB');
    expect(formatBytes(20 * 1024 ** 2)).toBe('20 MB');
    expect(formatBytes(2 * 1024 ** 3)).toBe('2 GB');
    expect(formatBytes(53687091200)).toBe('50 GB');
  });
});

describe('a date of the page', () => {
  it('ends with the year, not with the mark of a year that would double a full stop', () => {
    // 2026-10-18 in every time zone of the day.
    const day = Date.UTC(2026, 9, 18, 12) / 1000;
    expect(dateOf(day, 'ru')).toMatch(/2026$/);
    expect(dateOf(day, 'uk')).toMatch(/2026$/);
    expect(dateOf(day, 'en')).toMatch(/2026$/);
  });
});

describe('a key of activation', () => {
  it('is read the way the server reads it', () => {
    expect(normalizeKey('7k2m-abcd-efgh-jkmn-pqrs')).toBe('7K2M-ABCD-EFGH-JKMN-PQRS');
    expect(normalizeKey(' 7K2M ABCD EFGH JKMN PQRo ')).toBe('7K2M-ABCD-EFGH-JKMN-PQR0');
    expect(normalizeKey('ILUU-0000-0000-0000-0000')).toBe('11VV-0000-0000-0000-0000');
    for (const bad of ['', '7K2M-ABCD', '7K2M-ABCD-EFGH-JKMN-PQRS-T', '7K2M-ABCD-EFGH-JKMN-PQR!']) {
      expect(normalizeKey(bad), bad).toBeNull();
    }
  });

  it('refuses anything but ASCII, as the module does, before the upper case', () => {
    // Upper-cased, `ß` would be `SS` and `ı` would be `I`, then `1`: a key of 20 again.
    for (const bad of ['7K2M-ABCD-EFGH-JKMN-PQß', '7K2M-ABCD-EFGH-JKMN-PQRı', '7K2M-ABCD-EFGH-JKMN-PQRЖ', '7K2M-ABCD-EFGH-JKMN-PQRS ']) {
      expect(normalizeKey(bad), bad).toBeNull();
    }
  });
});

describe('the form of the deletion', () => {
  it('sends only with the word DELETE and, under a lock, its secret', () => {
    const open = emptyDeleteForm(false);
    // The trusted people are asked to drop their shares unless unticked (C5).
    expect(SHARES_REVOCABLE).toBe(true);
    expect(open.revokeShares).toBe(true);
    expect(open.wipeDevice).toBe(false);
    expect(deleteReady(open)).toBe(false);
    expect(deleteReady({ ...open, word: 'delete' })).toBe(false);
    expect(deleteReady({ ...open, word: ' DELETE ' })).toBe(true);
    const locked = emptyDeleteForm(true);
    expect(deleteReady({ ...locked, word: 'DELETE' })).toBe(false);
    expect(deleteReady({ ...locked, word: 'DELETE', secret: '4821' })).toBe(true);
  });

  it('names the words of a refusal by its code', () => {
    expect(errorKeyOf('lock_wrong')).toBe('cloud_err_lock_wrong');
    expect(errorKeyOf('transport')).toBe('cloud_err_no_host');
    expect(errorKeyOf('key_used')).toBe('cloud_err_key_used');
    expect(errorKeyOf('internal')).toBeNull();
  });
});
