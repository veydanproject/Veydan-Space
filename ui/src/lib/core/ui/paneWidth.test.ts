// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { KEY_STEP, clampWidth, dragWidth, keyWidth, sideMax } from './paneWidth';

describe('paneWidth', () => {
  it('clamps to the bounds and rounds', () => {
    expect(clampWidth(100, 320, 720)).toBe(320);
    expect(clampWidth(900, 320, 720)).toBe(720);
    expect(clampWidth(420.6, 320, 720)).toBe(421);
    expect(clampWidth(Number.NaN, 320, 720)).toBe(320);
    // A pane too narrow for its own bounds: the least wins.
    expect(clampWidth(500, 320, 280)).toBe(320);
  });

  it('caps the side column at 720 and 60 % of the pane', () => {
    expect(sideMax(2000, 320, 720)).toBe(720);
    expect(sideMax(1000, 320, 720)).toBe(600);
    expect(sideMax(400, 320, 720)).toBe(320);
    expect(sideMax(0, 320, 720)).toBe(720);
  });

  it('follows the pointer from where the drag began', () => {
    expect(dragWidth(340, 1000, 920, 'right')).toBe(420);
    expect(dragWidth(340, 1000, 1040, 'right')).toBe(300);
    expect(dragWidth(240, 240, 300, 'left')).toBe(300);
  });

  it('steps by the keyboard and stops at the bounds', () => {
    expect(keyWidth(340, 'ArrowLeft', 'right', 320, 720)).toBe(340 + KEY_STEP);
    expect(keyWidth(340, 'ArrowRight', 'right', 320, 720)).toBe(324);
    expect(keyWidth(330, 'ArrowRight', 'right', 320, 720)).toBe(320);
    expect(keyWidth(710, 'ArrowLeft', 'right', 320, 720)).toBe(720);
    expect(keyWidth(340, 'ArrowRight', 'left', 320, 720)).toBe(356);
    expect(keyWidth(500, 'Home', 'right', 320, 720)).toBe(320);
    expect(keyWidth(500, 'End', 'right', 320, 600)).toBe(600);
    expect(keyWidth(500, 'Enter', 'right', 320, 720)).toBeNull();
  });
});
