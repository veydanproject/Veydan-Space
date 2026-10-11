// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { bannerOf, fullestPool, largest, usageApi, type CloudUsage } from '$lib/cloud/usage';

function usage(level: number, onCloud = true): CloudUsage {
  return {
    pools: [
      { pool: 'text', used: 1, limit: 100, percent: 1, level: 0 },
      { pool: 'files', used: level, limit: 100, percent: level, level: level >= 100 ? 100 : level >= 90 ? 90 : level >= 70 ? 70 : 0 },
    ],
    level: level >= 100 ? 100 : level >= 90 ? 90 : level >= 70 ? 70 : 0,
    categories: [
      { id: 'journal', pool: 'text', bytes: 1, objects: 3 },
      { id: 'large_files', pool: 'files', bytes: 400, objects: 2 },
      { id: 'files', pool: 'files', bytes: 1500, objects: 9 },
      { id: 'other', pool: 'files', bytes: 0, objects: 0 },
    ],
    by_owner: false,
    profile_files_estimate: 0,
    on_cloud: onCloud,
  };
}

describe('the banner of the room of Veydan Cloud', () => {
  it('warns at 90, says full at 100, and only on a device that syncs through the cloud', () => {
    expect(bannerOf(null)).toBe('none');
    expect(bannerOf(usage(70))).toBe('none');
    expect(bannerOf(usage(90))).toBe('warn');
    expect(bannerOf(usage(100))).toBe('full');
    expect(bannerOf(usage(100, false))).toBe('none');
    expect(fullestPool(usage(95))?.pool).toBe('files');
  });

  it('lists what takes the room, the largest first, and leaves out the empty', () => {
    expect(largest(usage(100)).map((c) => c.id)).toEqual(['files', 'large_files', 'journal']);
    expect(largest(null)).toEqual([]);
  });

  it('finds no host in a plain browser', async () => {
    await expect(usageApi.usage()).rejects.toMatchObject({ code: 'no_host' });
  });
});
