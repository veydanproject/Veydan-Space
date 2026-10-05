// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { clampPercent, isCancelled, progressLine, versionLine } from './format';

describe('the Tor install block lines', () => {
  it('shows megabytes and percent, and the percent alone without a size', () => {
    expect(progressLine({ downloaded: 5 * 1024 * 1024, total: 20 * 1024 * 1024, percent: 25 })).toBe('5 MB / 20 MB · 25%');
    expect(progressLine({ downloaded: 100, total: 0, percent: 0 })).toBe('0%');
  });

  it('keeps the percent of the bar inside 0..100', () => {
    expect(clampPercent(140)).toBe(100);
    expect(clampPercent(-3)).toBe(0);
    expect(clampPercent(Number.NaN)).toBe(0);
    expect(clampPercent(null)).toBe(0);
    expect(clampPercent(33.4)).toBe(33);
  });

  it('joins the versions that are known', () => {
    expect(versionLine('14.5.8', '0.4.8.17')).toBe('14.5.8 · tor 0.4.8.17');
    expect(versionLine('14.5.8', null)).toBe('14.5.8');
    expect(versionLine(null, '0.4.8.17')).toBe('tor 0.4.8.17');
    expect(versionLine(null, null)).toBe('');
  });

  it('recognises a cancelled download', () => {
    expect(isCancelled('download cancelled')).toBe(true);
    expect(isCancelled('connection reset')).toBe(false);
  });
});
