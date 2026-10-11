// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The cards of the bot drawn in its chat right now. A card open in the chat
// already says what its banner says and has the same button: the banner of
// that card stays away while the card is on the screen, so one message is
// not shown twice with two «Это не я — отменить».

class CardsOnScreen {
  /** How many times each key is on the screen: the id of the card, and the id of its request. */
  private shown = $state<Record<string, number>>({});

  /** A card came to the screen; returns the way to say it left. */
  show(...keys: (string | null | undefined)[]): () => void {
    const mine = keys.filter((k): k is string => !!k);
    const next = { ...this.shown };
    for (const k of mine) next[k] = (next[k] ?? 0) + 1;
    this.shown = next;
    return () => {
      const left = { ...this.shown };
      for (const k of mine) {
        if ((left[k] ?? 0) <= 1) delete left[k];
        else left[k] -= 1;
      }
      this.shown = left;
    };
  }

  /** Whether a card with one of these keys is on the screen. */
  has(...keys: (string | null | undefined)[]): boolean {
    return keys.some((k) => !!k && (this.shown[k] ?? 0) > 0);
  }
}

export const cardsOnScreen = new CardsOnScreen();
