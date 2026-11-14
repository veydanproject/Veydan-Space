// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// «Запросить с сервера» under a search of a chat that found nothing: the
// relays hold no words to search (E2E), so the older messages of the chat
// are brought to the device the way «Загрузить раньше» brings them, and the
// search is asked again over them. One press asks at most `ASK_ROUNDS` times
// and stops as soon as a new match shows, the relays have nothing older, or
// the press no longer stands (cancelled, another question, another chat).

/** Asks of the relays one press makes at most. */
export const ASK_ROUNDS = 3;

/** How a press ended. */
export type AskOutcome = 'found' | 'end' | 'rounds' | 'stopped';

export interface AskSteps {
  /** The time of the oldest message of the chat on the device; null: it has none. */
  floor(): Promise<number | null>;
  /** One ask of the relays for what is older than `before`; whether they have nothing older now. */
  ask(before: number): Promise<{ end: boolean }>;
  /** The messages of the chat older than `floor` now on the device: how many, and the oldest one's time. */
  under(floor: number | null): Promise<{ count: number; floor: number | null }>;
  /** The search asked again: how many it finds. */
  search(): Promise<number>;
  /** The press still stands. */
  live(): boolean;
  /** More older messages came: how many in all, by this press. */
  loaded(n: number): void;
  /** Now, in unix seconds: an empty chat asks for what is older than now. */
  now(): number;
}

/** One press: `found` is how many the search found before it. */
export async function askRounds(steps: AskSteps, found: number, rounds = ASK_ROUNDS): Promise<AskOutcome> {
  let loaded = 0;
  for (let round = 0; round < rounds; round++) {
    if (!steps.live()) return 'stopped';
    const floor = await steps.floor();
    // Reading down to the oldest held may take pages: cancelled meanwhile, the relays are not asked.
    if (!steps.live()) return 'stopped';
    const { end } = await steps.ask(floor ?? steps.now());
    const under = await steps.under(floor);
    if (!steps.live()) return 'stopped';
    if (under.count > 0) steps.loaded((loaded += under.count));
    const now = await steps.search();
    if (!steps.live()) return 'stopped';
    if (now > found) return 'found';
    if (end) return 'end';
  }
  return 'rounds';
}

/** A group's windows in a row the relays answered empty that are its end, for «Загрузить раньше» and this button alike. */
export const GROUP_EMPTY_END = 2;

/**
 * A group's empty windows in a row after one more ask: `got` events came of
 * the relays (null: the ask failed — no relay answered, no link), `added`
 * messages came to view, `linked` the link was up when the answer came. A
 * failed ask, or an empty window while the link was lost, says nothing of
 * the end: the count stays, and so does the button.
 */
export function windowsEmpty(before: number, got: number | null, linked: boolean, added = 0): number {
  if (got === null) return before;
  if (got > 0 || added > 0) return 0;
  return linked ? before + 1 : before;
}

/** The runtime's word for an ask no relay took (`no relay answered`, `no relay connected`): said as «no connection». */
export function relaysUnreached(error: string): boolean {
  return /\bno relay (answered|connected)\b/.test(error);
}
