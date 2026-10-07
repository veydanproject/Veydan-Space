// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The sounds of a call, made by Web Audio: no file ships with the app and
// nothing is fetched. The ring of a call that comes in, the tone the caller
// hears while the peer's device rings, the short beeps when a call ends.
// The voice itself never passes here: the engine plays it in Rust.

/** Something that sounds until it is stopped. */
export type Stop = () => void;

const noop: Stop = () => {};

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
 * One note: a sine (with a softer octave above for warmth) that rises in a
 * few milliseconds and fades, so that nothing clicks.
 */
function note(a: AudioContext, out: AudioNode, at: number, freq: number, length: number, volume: number, harmonic = 0.18) {
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

/** A steady tone, its ends softened. */
function tone(a: AudioContext, out: AudioNode, at: number, freq: number, length: number, volume: number) {
  const osc = a.createOscillator();
  const gain = a.createGain();
  osc.type = 'sine';
  osc.frequency.value = freq;
  gain.gain.setValueAtTime(0, at);
  gain.gain.linearRampToValueAtTime(volume, at + 0.02);
  gain.gain.setValueAtTime(volume, at + length - 0.03);
  gain.gain.linearRampToValueAtTime(0, at + length);
  osc.connect(gain).connect(out);
  osc.start(at);
  osc.stop(at + length + 0.05);
}

/**
 * Plays `cycle` (a pattern starting at the given time) every `period`
 * seconds until stopped. Each cycle is scheduled a little ahead, so that a
 * busy page does not stutter it; stopping fades out what was scheduled.
 */
function repeat(period: number, cycle: (a: AudioContext, out: AudioNode, at: number) => void): Stop {
  const a = audio();
  if (!a) return noop;
  const out = a.createGain();
  out.connect(a.destination);
  let next = a.currentTime + 0.05;
  const plan = () => {
    while (next < a.currentTime + 1.5) {
      cycle(a, out, next);
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
    out.gain.setTargetAtTime(0, a.currentTime, 0.03);
    setTimeout(() => out.disconnect(), 400);
  };
}

/** A call comes in: a rising chime of three notes, twice, then a pause. */
export function ringtone(): Stop {
  // E5, A5, C#6: a bright major chord, played as an arpeggio.
  const chord = [659.25, 880, 1108.73];
  return repeat(3.2, (a, out, at) => {
    for (const round of [0, 0.62]) {
      chord.forEach((f, i) => note(a, out, at + round + i * 0.13, f, 0.55, 0.16));
    }
  });
}

/** I call and the peer's device rings: the tone of a telephone line, 425 Hz, one second in four. */
export function ringback(): Stop {
  return repeat(4, (a, out, at) => tone(a, out, at, 425, 1, 0.07));
}

/** The call is over: three short beeps, once. */
export function hangupTone(): void {
  const a = audio();
  if (!a) return;
  const out = a.createGain();
  out.connect(a.destination);
  const at = a.currentTime + 0.03;
  for (let i = 0; i < 3; i++) tone(a, out, at + i * 0.32, 425, 0.18, 0.07);
  setTimeout(() => out.disconnect(), 1500);
}
