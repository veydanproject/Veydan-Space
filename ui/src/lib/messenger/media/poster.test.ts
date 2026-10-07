// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { frameTime, posterUrl } from './poster';

describe('the frame of a video', () => {
  it('is taken a little after the start, never at zero nor past a second', () => {
    expect(frameTime(0)).toBe(0.1);
    expect(frameTime(NaN)).toBe(0.1);
    expect(frameTime(Infinity)).toBe(0.1);
    expect(frameTime(4)).toBeCloseTo(0.4);
    expect(frameTime(600)).toBe(1);
  });

  it('shows as a JPEG', () => {
    expect(posterUrl({ jpeg: '/9j/', width: 2, height: 1 })).toBe('data:image/jpeg;base64,/9j/');
  });
});
