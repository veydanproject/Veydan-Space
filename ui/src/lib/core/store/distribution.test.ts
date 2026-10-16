// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { beforeEach, describe, expect, it, vi } from 'vitest';

const hostInfo = vi.fn();
vi.mock('$lib/core/api', () => ({ api: { system: { hostInfo } } }));

async function fresh() {
  vi.resetModules();
  return import('./distribution.svelte');
}

describe('where the app came from', () => {
  beforeEach(() => {
    hostInfo.mockReset();
  });

  it('is not from a store before it is asked', async () => {
    const { distribution } = await fresh();
    expect(distribution.store).toBeNull();
    expect(distribution.fromStore).toBe(false);
  });

  it('is from Play when the shell says so, and asks only once', async () => {
    hostInfo.mockResolvedValue({ store: 'play' });
    const { distribution, loadDistribution } = await fresh();
    await loadDistribution();
    await loadDistribution();
    expect(distribution.store).toBe('play');
    expect(distribution.fromStore).toBe(true);
    expect(hostInfo).toHaveBeenCalledTimes(1);
  });

  it('is not from a store when the shell says null', async () => {
    hostInfo.mockResolvedValue({ store: null });
    const { distribution, loadDistribution } = await fresh();
    await loadDistribution();
    expect(distribution.fromStore).toBe(false);
  });

  it('swallows an error: a host without the command stays out of the store', async () => {
    hostInfo.mockImplementation(() => {
      throw new Error('no command');
    });
    const { distribution, loadDistribution } = await fresh();
    await loadDistribution();
    expect(distribution.fromStore).toBe(false);
  });
});
