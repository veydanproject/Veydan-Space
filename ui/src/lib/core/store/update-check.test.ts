// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it, vi } from 'vitest';
import { UpdateCheckStore } from './update-check.svelte';
import type { UpdateCheck } from '$lib/core/types';

const answer = (latest: string | null, available: boolean): UpdateCheck => ({
  current: '5.0.0',
  latest,
  available,
  checked_at: latest ? 100 : null,
  page: latest ? `https://github.com/veydanproject/Veydan-Notes/releases/tag/v${latest}` : null,
  skipped: null,
});

describe('the update check of a phone', () => {
  it('runs the check of the start once, silently, as not asked by the user', async () => {
    const ask = vi.fn(async () => answer('5.1.0', true));
    const store = new UpdateCheckStore(ask);
    await store.auto();
    await store.auto();
    expect(ask).toHaveBeenCalledTimes(1);
    expect(ask).toHaveBeenCalledWith(false);
    expect(store.status).toBe('available');
    expect(store.latest).toBe('5.1.0');
  });

  it('stays quiet when the check of the start fails or found nothing yet', async () => {
    const failing = new UpdateCheckStore(async () => {
      throw new Error('offline');
    });
    await failing.auto();
    expect([failing.status, failing.error]).toEqual(['idle', '']);

    const skipped = new UpdateCheckStore(async () => ({ ...answer(null, false), skipped: 'metered' }));
    await skipped.auto();
    expect(skipped.status).toBe('idle');
  });

  it('asks the network when the user checks and reports a failure', async () => {
    const ask = vi.fn(async (manual: boolean) => (manual ? answer('5.0.0', false) : answer(null, false)));
    const store = new UpdateCheckStore(ask);
    await store.check();
    expect(ask).toHaveBeenCalledWith(true);
    expect(store.status).toBe('upToDate');

    const failing = new UpdateCheckStore(async () => {
      throw 'HTTP 404';
    });
    await failing.check();
    expect(failing.status).toBe('error');
    expect(failing.error).toContain('404');
  });
});
