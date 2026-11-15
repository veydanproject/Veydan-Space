// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { hasText } from './selectable';

describe('hasText', () => {
  it('takes a message with words, a caption too', () => {
    expect(hasText({ text: 'hello', deleted: false })).toBe(true);
    expect(hasText({ text: ' ', deleted: false })).toBe(true);
  });

  it('leaves a message with no words alone: a picture, a card, a sticker', () => {
    expect(hasText({ text: null, deleted: false })).toBe(false);
    expect(hasText({ text: '', deleted: false })).toBe(false);
  });

  it('leaves a deleted message alone, whatever it kept', () => {
    expect(hasText({ text: 'hello', deleted: true })).toBe(false);
    expect(hasText({ text: null, deleted: true })).toBe(false);
  });
});
