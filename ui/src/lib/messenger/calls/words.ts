// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What a call is called on the screen: the phase of the one under way, how
// one ended, its line in the chat (the runtime's system row `sys:call:<id>`,
// its facts in `media`), the refusals of the call commands. Pure, so the
// words are tested without a screen.

import type { MessengerMessage } from '../api';
import type { CallDirection, CallMedia, CallOutcome, CallView, CallVia } from '../generated/calls';

type Translate = (key: string, params?: Record<string, string>) => string;

/** `m:ss`, or `h:mm:ss` from an hour on. */
export function clock(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, '0');
  return h ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${m}:${ss}`;
}

/**
 * How long a call was, for its line in the chat: words of the language
 * ("12 min, 34 sec", short) where the platform has `Intl.DurationFormat`,
 * the clock elsewhere. The clock beside a time of day would read as one.
 */
export function duration(secs: number, tag: string): string {
  const s = Math.max(0, Math.floor(secs));
  const Format = (Intl as unknown as { DurationFormat?: new (tag: string, o: object) => { format(d: object): string } }).DurationFormat;
  if (!Format) return clock(s);
  const parts = { hours: Math.floor(s / 3600), minutes: Math.floor((s % 3600) / 60), seconds: s % 60 };
  try {
    return new Format(tag, { style: 'short' }).format(parts.hours ? { hours: parts.hours, minutes: parts.minutes } : parts.minutes ? { minutes: parts.minutes, seconds: parts.seconds } : { seconds: parts.seconds });
  } catch {
    return clock(s);
  }
}

/** How long the call has been talking, at `now` (unix seconds); 0 before it was answered. */
export function elapsed(call: Pick<CallView, 'answered_at'>, now: number): number {
  return call.answered_at ? Math.max(0, now - call.answered_at) : 0;
}

/**
 * The words under the name while the call is not talking; `null` while it
 * is (the screen shows the clock then). `reconnecting`: it was talking a
 * moment ago and its way is being restored.
 */
export function phaseKey(call: Pick<CallView, 'phase' | 'media' | 'answered_at'>): string | null {
  switch (call.phase) {
    case 'outgoing': return 'msg_call_phase_outgoing';
    case 'incoming': return call.media === 'video' ? 'msg_call_phase_incoming_video' : 'msg_call_phase_incoming';
    case 'connecting': return 'msg_call_phase_connecting';
    case 'reconnecting': return 'msg_call_phase_reconnecting';
    case 'ended': return 'msg_call_ended';
    default: return null;
  }
}

/** How a call ended, as the screen says it for a moment after. */
export function endedKey(outcome: CallOutcome, direction: CallDirection): string {
  switch (outcome) {
    case 'declined': return 'msg_call_ended_declined';
    case 'busy': return 'msg_call_ended_busy';
    case 'missed': return direction === 'out' ? 'msg_call_ended_no_answer' : 'msg_call_ended_missed';
    case 'failed': return 'msg_call_ended_failed';
    case 'answered_elsewhere': return 'msg_call_ended_elsewhere';
    default: return 'msg_call_ended';
  }
}

/** The facts of a call's line in the chat, as the runtime wrote them. */
export interface CallLine {
  callId: string;
  direction: CallDirection;
  media: CallMedia;
  /** `null` while the call is under way (or the device went away in it). */
  outcome: Exclude<CallOutcome, 'answered_elsewhere'> | null;
  /** Seconds it was answered; `null` for one that never was. */
  duration: number | null;
  /** How the media went; known once the call is over. */
  via: CallVia | null;
  startedAt: number;
}

const OUTCOMES = new Set(['missed', 'declined', 'busy', 'ended', 'failed']);

/** The runtime's line of a call in a chat (`feed.rs` of messenger-calls). */
export function isCallLine(m: MessengerMessage): boolean {
  return m.content_type === 'system' && m.text === 'call' && m.id.startsWith('sys:call:');
}

/** The line of a call, or `null` when the message is no such line. */
export function callLineOf(m: MessengerMessage): CallLine | null {
  if (!isCallLine(m)) return null;
  const d = (m.media ?? {}) as Record<string, unknown>;
  const direction: CallDirection = d.direction === 'in' ? 'in' : 'out';
  const outcome = typeof d.outcome === 'string' && OUTCOMES.has(d.outcome) ? (d.outcome as CallLine['outcome']) : null;
  return {
    callId: typeof d.call_id === 'string' ? d.call_id : m.id.slice('sys:call:'.length),
    direction,
    media: d.media === 'video' ? 'video' : 'audio',
    outcome,
    duration: typeof d.duration_secs === 'number' ? d.duration_secs : null,
    via: d.via === 'relay' || d.via === 'direct' ? d.via : null,
    startedAt: typeof d.started_at === 'number' ? d.started_at : m.created_at,
  };
}

/**
 * A call of the peer that I did not take: rang out, or came while I was in
 * another call. The chat shows it so that it is seen.
 */
export function isMissed(line: CallLine): boolean {
  return line.direction === 'in' && (line.outcome === 'missed' || line.outcome === 'busy');
}

/**
 * What the line says: a title (the kind of call) and the details after it
 * (how long, or why it did not happen, and that it went through a relay).
 * `live`: this very call is under way now.
 */
export function callLineWords(line: CallLine, tr: Translate, live = false, howLong: (secs: number) => string = clock): { title: string; detail: string | null } {
  const video = line.media === 'video' ? '_video' : '';
  const title = isMissed(line)
    ? tr(`msg_call_line_missed${video}`)
    : tr(`msg_call_line_${line.direction}${video}`);
  const parts: string[] = [];
  if (live) parts.push(tr('msg_call_detail_now'));
  else if (line.outcome === 'ended' && line.duration != null) parts.push(howLong(line.duration));
  else if (line.outcome === 'missed' && line.direction === 'out') parts.push(tr('msg_call_detail_no_answer'));
  else if (line.outcome === 'declined') parts.push(tr('msg_call_detail_declined'));
  else if (line.outcome === 'busy') parts.push(tr('msg_call_detail_busy'));
  else if (line.outcome === 'failed') parts.push(tr('msg_call_detail_failed'));
  if (!live && line.outcome === 'ended' && line.via === 'relay') parts.push(tr('msg_call_detail_relay'));
  return { title, detail: parts.length ? parts.join(' · ') : null };
}

/**
 * A refusal of a call command, in words. The runtime's refusals are its
 * own sentences, not codes yet: the known ones are recognised here.
 */
export function callErrorText(e: unknown, tr: Translate): string {
  const text = errorString(e);
  if (/contacts only|own key/i.test(text)) return tr('msg_call_err_contacts');
  if (/call is under way/i.test(text)) return tr('msg_call_err_busy');
  if (/no media engine/i.test(text)) return tr('msg_call_unavailable');
  if (/not logged in|messenger identity/i.test(text)) return tr('msg_call_err_locked');
  // My camera failed: the call itself goes on, without my video.
  if (/no camera/i.test(text)) return tr('msg_call_err_no_camera');
  if (/camera|\/dev\/video|uncompressed format/i.test(text)) return tr('msg_call_err_camera', { error: text });
  if (/screen|window|no display/i.test(text)) return tr('msg_call_err_screen', { error: text });
  return tr('msg_call_err_other', { error: text });
}

function errorString(e: unknown): string {
  if (e != null && typeof e === 'object' && 'message' in e && typeof (e as { message: unknown }).message === 'string') {
    return (e as { message: string }).message;
  }
  if (e != null && typeof e === 'object' && 'code' in e) return String((e as { code: unknown }).code);
  return typeof e === 'string' ? e : String(e);
}
