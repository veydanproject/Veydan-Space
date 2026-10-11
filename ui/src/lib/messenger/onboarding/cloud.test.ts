// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { cloudOnboarding, hasAccount, mockCloud } from './cloud.svelte';

describe('Veydan Cloud in the onboarding of the messenger', () => {
  it('starts the free plan where the module answers, and offers its screen', async () => {
    let started = 0;
    mockCloud({
      cloud_activate_free: () => {
        started++;
        return { phase: 'solving' };
      },
    });
    await cloudOnboarding.start();
    expect(started).toBe(1);
    expect(cloudOnboarding.available).toBe(true);
    expect(cloudOnboarding.phase).toBe('solving');
  });

  it('goes on without a screen where the product has no cloud', async () => {
    mockCloud({
      cloud_activate_free: () => {
        throw { code: 'unknown_command', message: 'cloud_activate_free' };
      },
    });
    await cloudOnboarding.start();
    expect(cloudOnboarding.available).toBe(false);
  });

  it('sees an account, live or kept, before the switch to the own servers', async () => {
    expect(hasAccount(null)).toBe(false);
    expect(hasAccount({ account: null, kept: null })).toBe(false);
    expect(hasAccount({ account: { plan: 'free' } })).toBe(true);
    expect(hasAccount({ account: null, kept: { plan: 'free' } })).toBe(true);
    mockCloud({ cloud_status: () => ({ account: { plan: 'free' }, kept: null, servers: 'veydan' }) });
    expect(hasAccount(await cloudOnboarding.seen())).toBe(true);
    mockCloud({
      cloud_status: () => {
        throw { code: 'unknown_command', message: 'cloud_status' };
      },
    });
    expect(await cloudOnboarding.seen()).toBeNull();
  });
});
