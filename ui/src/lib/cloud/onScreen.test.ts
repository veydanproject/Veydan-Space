// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { cardsOnScreen } from '$lib/cloud/onScreen.svelte';

describe('the cards of the bot on the screen', () => {
  it('knows a card by its id and by its request while it is shown, and forgets it after', () => {
    expect(cardsOnScreen.has('m1', 'r1')).toBe(false);
    const leave = cardsOnScreen.show('m1', 'r1');
    expect(cardsOnScreen.has('m1')).toBe(true);
    expect(cardsOnScreen.has('other', 'r1')).toBe(true);
    expect(cardsOnScreen.has('other', null)).toBe(false);
    leave();
    expect(cardsOnScreen.has('m1', 'r1')).toBe(false);
  });

  it('keeps a card that is drawn twice until both are gone', () => {
    const first = cardsOnScreen.show('m2', null);
    const second = cardsOnScreen.show('m2');
    first();
    expect(cardsOnScreen.has('m2')).toBe(true);
    second();
    expect(cardsOnScreen.has('m2')).toBe(false);
  });
});
