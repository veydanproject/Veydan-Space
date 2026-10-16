// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The journal of calls (the section «Звонки») as the list shows it: the
// calls that are over, newest first, the calls of one chat in a row of
// one day and of one kind folded into one row with their count, as a
// phone's journal does, under the headings of their days. Pure, so it is
// tested without a screen.

import type { CallLogEntry } from '../generated/calls';
import { isMissed, type CallLine } from './words';

/** The facts of an entry as the line of its chat has them, for the words of `words.ts`. */
export function lineOf(e: CallLogEntry): CallLine {
  return {
    kind: e.kind,
    callId: e.call_id,
    direction: e.direction,
    media: e.media,
    outcome: e.outcome === 'answered_elsewhere' ? 'ended' : e.outcome,
    duration: e.duration_secs,
    via: e.outcome === 'ended' ? (e.via_relay ? 'relay' : 'direct') : null,
    startedAt: e.started_at,
    startedBy: e.started_by,
    participants: e.participants,
  };
}

/** A call between two that rang here and nobody took: red in the list, counted on the section. */
export function isLogMissed(e: CallLogEntry): boolean {
  return e.kind === 'dm' && isMissed(lineOf(e));
}

/** One row of the list: the newest call of the fold and every call in it, newest first. */
export interface LogRow {
  key: string;
  head: CallLogEntry;
  calls: CallLogEntry[];
  missed: boolean;
}

/** What folds calls into one row: the same chat, the same day, the same way and the same missed or not. */
function foldKey(e: CallLogEntry, dayOf: (unix: number) => string): string {
  return `${e.chat_id}|${dayOf(e.started_at)}|${e.direction}|${isLogMissed(e) ? 'm' : ''}`;
}

/** The rows of `entries` (newest first): neighbours of one fold become one row. */
export function foldLog(entries: CallLogEntry[], dayOf: (unix: number) => string): LogRow[] {
  const rows: LogRow[] = [];
  let lastKey = '';
  for (const e of entries) {
    const key = foldKey(e, dayOf);
    const last = rows[rows.length - 1];
    if (last && key === lastKey) last.calls.push(e);
    else rows.push({ key: e.call_id, head: e, calls: [e], missed: isLogMissed(e) });
    lastKey = key;
  }
  return rows;
}

/** The rows under the headings of their days, in the order they come. */
export function logSections(rows: LogRow[], dayOf: (unix: number) => string): { day: string; at: number; rows: LogRow[] }[] {
  const out: { day: string; at: number; rows: LogRow[] }[] = [];
  for (const r of rows) {
    const day = dayOf(r.head.started_at);
    const last = out[out.length - 1];
    if (last && last.day === day) last.rows.push(r);
    else out.push({ day, at: r.head.started_at, rows: [r] });
  }
  return out;
}
