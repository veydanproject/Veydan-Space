// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The sounds of a call, made by Web Audio: no file ships with the app and
// nothing is fetched. The ring of a call that comes in; the motif the caller
// hears while the peer's device rings; a short chime when the call
// connects; a falling one when it ends; a "no" of two falling notes when the
// peer declines, is busy or does not answer. The voice itself never passes
// here: the engine plays it in Rust.
//
// The caller's sounds are one family in A major, played by a soft
// mallet-like voice (a sine with a little of its octave and twelfth, a quick
// rise and a long fall) through a gentle low-pass and a faint echo: the
// motif rises (A, C#, E), "connected" leaps up a fourth, "ended" falls the
// motif back down; "busy" alone is minor, so it is told apart at once.

/** Something that sounds until it is stopped. */
export type Stop = () => void;

/** One note of a motif: when (seconds from its start), the pitch, how long it rings, how loud. */
export interface Note {
  at: number;
  freq: number;
  length: number;
  volume: number;
}

const noop: Stop = () => {};

// Pitches, equal temperament from A4 = 440 Hz.
const A4 = 440;
const B4 = 493.88;
const CS5 = 554.37;
const D5 = 587.33;
const E5 = 659.25;
const A5 = 880;

/** The motif the caller hears while the peer's device rings: A, C#, E rising, the last one held. */
export const RINGBACK: readonly Note[] = [
  { at: 0, freq: A4, length: 0.55, volume: 0.07 },
  { at: 0.17, freq: CS5, length: 0.55, volume: 0.065 },
  { at: 0.34, freq: E5, length: 1.1, volume: 0.07 },
];
/** How often the motif comes back, seconds. */
export const RINGBACK_PERIOD = 3;

/** The call connected: a short leap up a fourth. */
export const CONNECTED: readonly Note[] = [
  { at: 0, freq: E5, length: 0.28, volume: 0.06 },
  { at: 0.09, freq: A5, length: 0.5, volume: 0.065 },
];

/** The call ended: the motif falling back, E, C#, A, the last one fading long. */
export const ENDED: readonly Note[] = [
  { at: 0, freq: E5, length: 0.3, volume: 0.065 },
  { at: 0.13, freq: CS5, length: 0.3, volume: 0.06 },
  { at: 0.26, freq: A4, length: 0.75, volume: 0.065 },
];

/** Declined, busy or no answer: a minor third falling, twice ("no, no"). */
export const BUSY: readonly Note[] = [
  { at: 0, freq: D5, length: 0.2, volume: 0.065 },
  { at: 0.17, freq: B4, length: 0.32, volume: 0.06 },
  { at: 0.55, freq: D5, length: 0.2, volume: 0.065 },
  { at: 0.72, freq: B4, length: 0.45, volume: 0.06 },
];

/** How long a motif sounds, its last note's tail included. */
export function motifLength(notes: readonly Note[]): number {
  return Math.max(0, ...notes.map((n) => n.at + n.length));
}

let ctx: AudioContext | null = null;

function audio(): AudioContext | null {
  if (typeof window === 'undefined') return null;
  const Ctor = window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
  if (!Ctor) return null;
  try {
    ctx ??= new Ctor();
    // A webview may start it suspended until the page was touched; a call is reason enough.
    if (ctx.state === 'suspended') ctx.resume().catch(() => {});
    return ctx;
  } catch {
    return null;
  }
}

/**
 * Where the notes go: a gentle low-pass (nothing sharp reaches a small
 * speaker) and a faint echo, so the notes bloom a little instead of
 * stopping dry. `release` fades it out and lets it go.
 */
function bus(a: AudioContext): { input: GainNode; release: (fade?: number) => void } {
  const input = a.createGain();
  const soften = a.createBiquadFilter();
  soften.type = 'lowpass';
  soften.frequency.value = 3200;
  soften.Q.value = 0.5;
  const delay = a.createDelay(1);
  delay.delayTime.value = 0.19;
  const feedback = a.createGain();
  feedback.gain.value = 0.22;
  const wet = a.createGain();
  wet.gain.value = 0.16;
  input.connect(soften);
  input.connect(delay);
  delay.connect(feedback).connect(delay);
  delay.connect(wet).connect(soften);
  soften.connect(a.destination);
  let released = false;
  return {
    input,
    release: (fade = 0.03) => {
      if (released) return;
      released = true;
      input.gain.setTargetAtTime(0, a.currentTime, fade);
      wet.gain.setTargetAtTime(0, a.currentTime, fade);
      setTimeout(() => {
        for (const node of [input, delay, feedback, wet, soften]) node.disconnect();
      }, 1200);
    },
  };
}

/**
 * One note of a soft mallet: a sine with a little of its octave and its
 * twelfth, rising in 10 ms (nothing clicks) and falling away over its length.
 */
function play(a: AudioContext, out: AudioNode, at: number, n: Note) {
  const gain = a.createGain();
  gain.gain.setValueAtTime(0, at);
  gain.gain.linearRampToValueAtTime(n.volume, at + 0.01);
  gain.gain.setTargetAtTime(0, at + 0.02, n.length / 3.2);
  gain.connect(out);
  for (const [mult, part] of [[1, 1], [2, 0.14], [3, 0.035]] as const) {
    const osc = a.createOscillator();
    osc.type = 'sine';
    osc.frequency.value = n.freq * mult;
    const g = a.createGain();
    g.gain.value = part;
    osc.connect(g).connect(gain);
    osc.start(at);
    osc.stop(at + n.length + 0.5);
  }
}

/** A motif once; its nodes are let go when it is over. */
function once(notes: readonly Note[]): void {
  const a = audio();
  if (!a) return;
  const out = bus(a);
  const at = a.currentTime + 0.03;
  for (const n of notes) play(a, out.input, at + n.at, n);
  setTimeout(() => out.release(0.2), (motifLength(notes) + 0.9) * 1000);
}

/**
 * Plays `cycle` (a pattern starting at the given time) every `period`
 * seconds until stopped. Each cycle is scheduled a little ahead, so that a
 * busy page does not stutter it; stopping fades out what was scheduled.
 * `softened`: through the low-pass and the echo of the caller's sounds.
 */
function repeat(period: number, cycle: (a: AudioContext, out: AudioNode, at: number) => void, softened = false): Stop {
  const a = audio();
  if (!a) return noop;
  let release: () => void;
  let input: AudioNode;
  if (softened) {
    const out = bus(a);
    input = out.input;
    release = () => out.release();
  } else {
    const out = a.createGain();
    out.connect(a.destination);
    input = out;
    release = () => {
      out.gain.setTargetAtTime(0, a.currentTime, 0.03);
      setTimeout(() => out.disconnect(), 400);
    };
  }
  let next = a.currentTime + 0.05;
  const plan = () => {
    while (next < a.currentTime + 1.5) {
      cycle(a, input, next);
      next += period;
    }
  };
  plan();
  const timer = setInterval(plan, 500);
  let stopped = false;
  return () => {
    if (stopped) return;
    stopped = true;
    clearInterval(timer);
    release();
  };
}

/**
 * One note of the incoming ring: a sine (with a softer octave above for
 * warmth) that rises in a few milliseconds and fades, so that nothing clicks.
 */
function chime(a: AudioContext, out: AudioNode, at: number, freq: number, length: number, volume: number, harmonic = 0.18) {
  const gain = a.createGain();
  gain.gain.setValueAtTime(0, at);
  gain.gain.linearRampToValueAtTime(volume, at + 0.012);
  gain.gain.setTargetAtTime(0, at + length * 0.45, length / 4);
  gain.connect(out);
  for (const [mult, part] of [[1, 1], [2, harmonic]] as const) {
    if (!part) continue;
    const osc = a.createOscillator();
    osc.type = 'sine';
    osc.frequency.value = freq * mult;
    const g = a.createGain();
    g.gain.value = part;
    osc.connect(g).connect(gain);
    osc.start(at);
    osc.stop(at + length + 0.3);
  }
}

/** A call comes in: a rising chime of three notes, twice, then a pause. */
export function ringtone(): Stop {
  // E5, A5, C#6: a bright major chord, played as an arpeggio.
  const chord = [659.25, 880, 1108.73];
  return repeat(3.2, (a, out, at) => {
    for (const round of [0, 0.62]) {
      chord.forEach((f, i) => chime(a, out, at + round + i * 0.13, f, 0.55, 0.16));
    }
  });
}

/** I call and the peer's device rings: the rising motif, every three seconds. */
export function ringback(): Stop {
  return repeat(RINGBACK_PERIOD, (a, out, at) => { for (const n of RINGBACK) play(a, out, at + n.at, n); }, true);
}

/** The call connected: once. */
export function connectedTone(): void {
  once(CONNECTED);
}

/** The talk is over (or I gave up calling): once. */
export function endTone(): void {
  once(ENDED);
}

/** The peer declined, is busy, or did not answer: once. */
export function busyTone(): void {
  once(BUSY);
}
