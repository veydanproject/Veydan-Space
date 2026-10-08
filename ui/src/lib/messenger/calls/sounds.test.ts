// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Which sound a call makes as it connects and ends, and the shape of the
// caller's sounds: short, soft, the motif every three seconds.

import { describe, expect, it } from 'vitest';
import type { CallOutcome, CallView } from '../generated/calls';
import type { EndedCall } from './callStore.svelte';
import { CONNECTED_FRESH_SECS, connectedSound, endSound } from './sounds';
import { BUSY, CONNECTED, ENDED, RINGBACK, RINGBACK_PERIOD, motifLength, type Note } from './tones';

function view(over: Partial<CallView> = {}): CallView {
  return {
    call_id: 'c1', peer: 'ab'.repeat(32), chat_id: 'dm:x', direction: 'out', media: 'audio', phase: 'ended',
    muted: false, started_at: 1000, nodes: [], video_local: false, video_screen: false, video_remote: false, ...over,
  };
}

const ended = (outcome: CallOutcome, over: Partial<CallView> = {}, local = false): EndedCall =>
  ({ call: view(over), outcome, duration: null, local });

describe('the sound of a call that ends', () => {
  it('falls after a talk, whoever ended it', () => {
    expect(endSound(ended('ended', { answered_at: 1005 }))).toBe('end');
    expect(endSound(ended('ended', { answered_at: 1005, direction: 'in' }, true))).toBe('end');
    expect(endSound(ended('failed', { answered_at: 1005 })), 'the line was lost').toBe('end');
  });

  it('says "no" when my call was declined, busy, not answered or not connected', () => {
    for (const o of ['declined', 'busy', 'missed', 'failed'] as const) expect(endSound(ended(o)), o).toBe('busy');
  });

  it('falls, not "no", when I gave up calling', () => {
    expect(endSound(ended('missed', {}, true))).toBe('end');
  });

  it('is silent for a call to me that never talked here', () => {
    for (const o of ['declined', 'missed', 'answered_elsewhere'] as const) {
      expect(endSound(ended(o, { direction: 'in' })), o).toBeNull();
      expect(endSound(ended(o, { direction: 'in' }, true)), `${o}, mine`).toBeNull();
    }
  });
});

describe('the chime of a call that connects', () => {
  it('rings when the call has just started to talk, not on a talk long under way', () => {
    expect(connectedSound(view({ phase: 'active', answered_at: 1000 }), 1003)).toBe(true);
    expect(connectedSound(view({ phase: 'active', answered_at: 1000 }), 1000 + CONNECTED_FRESH_SECS + 1)).toBe(false);
    expect(connectedSound(view({ phase: 'connecting', answered_at: 1000 }), 1001)).toBe(false);
  });
});

describe('the caller\'s sounds', () => {
  const all: [string, readonly Note[]][] = [['ringback', RINGBACK], ['connected', CONNECTED], ['ended', ENDED], ['busy', BUSY]];

  it('are soft: no note louder than a ringback of the line was', () => {
    for (const [name, notes] of all) for (const n of notes) expect(n.volume, name).toBeLessThanOrEqual(0.08);
  });

  it('are short, and the motif comes back every three seconds with a pause between', () => {
    expect(RINGBACK_PERIOD).toBe(3);
    expect(motifLength(RINGBACK)).toBeLessThan(RINGBACK_PERIOD / 2);
    expect(motifLength(CONNECTED)).toBeLessThan(1);
    expect(motifLength(ENDED)).toBeLessThan(1.2);
    expect(motifLength(BUSY)).toBeLessThan(1.5);
  });

  it('are more than one tone: the motif rises, the end falls', () => {
    const rises = (notes: readonly Note[]) => notes.every((n, i) => i === 0 || n.freq > notes[i - 1].freq);
    const falls = (notes: readonly Note[]) => notes.every((n, i) => i === 0 || n.freq < notes[i - 1].freq);
    expect(new Set(RINGBACK.map((n) => n.freq)).size).toBeGreaterThanOrEqual(3);
    expect(rises(RINGBACK)).toBe(true);
    expect(rises(CONNECTED)).toBe(true);
    expect(falls(ENDED)).toBe(true);
  });

  it('tell "busy" from "ended": other notes', () => {
    const pitches = (notes: readonly Note[]) => new Set(notes.map((n) => n.freq));
    const shared = [...pitches(BUSY)].filter((f) => pitches(ENDED).has(f));
    expect(shared).toEqual([]);
  });
});
