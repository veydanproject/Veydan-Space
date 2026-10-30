// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import {
  KEY_STEP, LIST_COVERS, LIST_DEFAULT, LIST_KEPT_COVERS, LIST_MAX, LIST_MIN,
  clampWidth, dragWidth, keepListWidth, keptListWidth, keyWidth, listKeyWidth, listMax, listWidth, sideMax,
} from './paneWidth';

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

describe('the list column', () => {
  it('folds to its covers under 180 and has words from 260', () => {
    expect(listWidth(72)).toBe(LIST_COVERS);
    expect(listWidth(179)).toBe(72);
    expect(listWidth(180)).toBe(LIST_MIN);
    expect(listWidth(259)).toBe(260);
    expect(listWidth(260)).toBe(260);
    expect(listWidth(431)).toBe(431);
  });

  it('jumps over the gap by the keyboard', () => {
    expect(listKeyWidth(72 + KEY_STEP, 72)).toBe(260);
    expect(listKeyWidth(260 - KEY_STEP, 260)).toBe(72);
    expect(listKeyWidth(276 - KEY_STEP, 276)).toBe(260);
    expect(listKeyWidth(260 + KEY_STEP, 260)).toBe(276);
    expect(listKeyWidth(72, 72)).toBe(72);
  });

  it('keeps the covers as the least the runtime takes, and reads them back', () => {
    expect(keepListWidth(72)).toBe(LIST_KEPT_COVERS);
    expect(keepListWidth(300.4)).toBe(300);
    expect(keptListWidth(LIST_KEPT_COVERS, 480)).toBe(72);
    expect(keptListWidth(null, 480)).toBe(LIST_DEFAULT);
    expect(keptListWidth(470, 440)).toBe(440);
    expect(keptListWidth(300, 480)).toBe(300);
  });

  it('leaves the conversation 400 px of the shell', () => {
    expect(listMax(1416)).toBe(LIST_MAX);
    expect(listMax(936)).toBe(440);
    expect(listMax(700)).toBe(LIST_MIN);
    expect(listMax(0)).toBe(LIST_MAX);
  });
});
