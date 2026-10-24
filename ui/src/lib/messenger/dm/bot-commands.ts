// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The «/» menu of a chat with a bot (Composer): which of the commands its
// profile lists it shows, and how the highlight moves.

import type { BotCommand } from '../generated/profile';

/** What the field says of a command: `/` and the beginning of a name, no space yet (lowercased); null for any other words. */
export function typedCommand(text: string): string | null {
  return /^\/\S*$/.test(text) ? text.slice(1).toLowerCase() : null;
}

/**
 * The commands the menu shows: those whose name begins with what was typed;
 * all of them when the «/» button asked and nothing is being typed; none
 * otherwise.
 */
export function commandsFor(commands: readonly BotCommand[], text: string, all: boolean): BotCommand[] {
  const q = typedCommand(text) ?? (all ? '' : null);
  return q === null ? [] : commands.filter((c) => c.name.startsWith(q));
}

/** The row `by` steps from `at` in a list of `n`, round from the last to the first and back. */
export function stepRow(at: number, by: number, n: number): number {
  return n > 0 ? (((at + by) % n) + n) % n : 0;
}
