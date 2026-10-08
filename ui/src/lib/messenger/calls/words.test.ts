// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { MessengerMessage } from '../api';
import { callErrorText, callLineOf, callLineWords, clock, duration, elapsed, endedKey, groupLineWords, isMissed, phaseKey } from './words';

/** The key itself, with its parameters: what the words are made of. */
const tr = (key: string, params?: Record<string, string>) => (params ? `${key}(${Object.values(params).join(',')})` : key);

function line(media: Record<string, unknown>, extra: Partial<MessengerMessage> = {}): MessengerMessage {
  return {
    id: 'sys:call:c1', chat_id: 'dm:x', direction: 'in', status: 'sent', content_type: 'system', text: 'call', sender_pubkey: '',
    reply_to: null, created_at: 100, edited_at: null, deleted: false, failure_reason: null, delivered_at: null, read_at: null,
    seen_by: [], reactions: [], media, ...extra,
  };
}

describe('the clock of a call', () => {
  it('counts minutes and seconds, hours from the first one on', () => {
    expect(clock(0)).toBe('0:00');
    expect(clock(65)).toBe('1:05');
    expect(clock(3599)).toBe('59:59');
    expect(clock(3600 + 62)).toBe('1:01:02');
    expect(clock(-3)).toBe('0:00');
  });

  it('says how long a call was in words where it can, by the clock where not', () => {
    const words = duration(754, 'en');
    if ('DurationFormat' in Intl) expect(words).toMatch(/12.*34/);
    else expect(words).toBe('12:34');
    // With hours, the words drop the seconds; the clock (no DurationFormat, as on
    // the CI's Node) keeps them.
    const long = duration(3 * 3600 + 120 + 5, 'en');
    if ('DurationFormat' in Intl) expect(long).not.toMatch(/5/);
    else expect(long).toBe('3:02:05');
  });

  it('runs from the answer only', () => {
    expect(elapsed({ answered_at: undefined }, 500)).toBe(0);
    expect(elapsed({ answered_at: 400 }, 500)).toBe(100);
  });
});

describe('the phase of a call', () => {
  it('has words until it talks, and a restored connection is not a new one', () => {
    expect(phaseKey({ phase: 'outgoing', media: 'audio' })).toBe('msg_call_phase_outgoing');
    expect(phaseKey({ phase: 'incoming', media: 'video' })).toBe('msg_call_phase_incoming_video');
    expect(phaseKey({ phase: 'connecting', media: 'audio' })).toBe('msg_call_phase_connecting');
    expect(phaseKey({ phase: 'reconnecting', media: 'audio', answered_at: 10 })).toBe('msg_call_phase_reconnecting');
    expect(phaseKey({ phase: 'active', media: 'audio', answered_at: 10 })).toBeNull();
  });

  it('ends in words of its own side', () => {
    expect(endedKey('missed', 'out')).toBe('msg_call_ended_no_answer');
    expect(endedKey('missed', 'in')).toBe('msg_call_ended_missed');
    expect(endedKey('answered_elsewhere', 'in')).toBe('msg_call_ended_elsewhere');
    expect(endedKey('ended', 'out')).toBe('msg_call_ended');
  });
});

describe('the line of a call in the chat', () => {
  it('is read from the runtime\'s system row only', () => {
    expect(callLineOf(line({}, { text: 'group_joined' }))).toBeNull();
    expect(callLineOf(line({}, { id: 'abc' }))).toBeNull();
    expect(callLineOf(line({}, { content_type: 'text' }))).toBeNull();
    const l = callLineOf(line({ call_id: 'c1', direction: 'out', media: 'video', outcome: 'ended', duration_secs: 75, via: 'relay', started_at: 90 }));
    expect(l).toEqual({
      kind: 'dm', callId: 'c1', direction: 'out', media: 'video', outcome: 'ended', duration: 75, via: 'relay', startedAt: 90, startedBy: null, participants: 0,
    });
  });

  it('reads the line of a group call: who started it, how many were in it', () => {
    const l = callLineOf(line({ kind: 'group', call_id: 'g1', direction: 'in', media: 'audio', started_by: 'b0b', participants: 3, outcome: 'ended', duration_secs: 600, started_at: 90 }, { sender_pubkey: 'b0b' }))!;
    expect(l).toMatchObject({ kind: 'group', callId: 'g1', startedBy: 'b0b', participants: 3, duration: 600 });
    const people = (n: number) => `people(${n})`;
    expect(groupLineWords(l, tr, { live: false, starter: 'Boris', people })).toEqual({ title: 'msg_gcall_line', detail: 'Boris · 10:00 · people(3)' });
    // On now: no length yet; mine: no starter's name.
    const now = callLineOf(line({ kind: 'group', media: 'video', participants: 2, outcome: null }))!;
    expect(groupLineWords(now, tr, { live: true, starter: null, people })).toEqual({ title: 'msg_gcall_line_video', detail: 'msg_gcall_detail_live · people(2)' });
    // Over without anybody else: nothing about people; a failed one says so.
    const alone = callLineOf(line({ kind: 'group', participants: 1, outcome: 'ended', duration_secs: 5 }))!;
    expect(groupLineWords(alone, tr, { live: false, starter: null, people }).detail).toBe('0:05');
    const failed = callLineOf(line({ kind: 'group', participants: 2, outcome: 'failed' }))!;
    expect(groupLineWords(failed, tr, { live: false, starter: null, people }).detail).toBe('msg_call_detail_failed');
    // A line that is not on and has no end (its call ended unseen): the title alone.
    expect(groupLineWords(callLineOf(line({ kind: 'group', outcome: null }))!, tr, { live: false, starter: null, people }).detail).toBeNull();
  });

  it('takes nothing it does not know', () => {
    const l = callLineOf(line({ outcome: 'answered_elsewhere', via: 'carrier pigeon', media: 'hologram' }))!;
    expect(l.outcome).toBeNull();
    expect(l.via).toBeNull();
    expect(l.media).toBe('audio');
    expect(l.callId).toBe('c1');
    expect(l.startedAt).toBe(100);
  });

  it('says how the call went', () => {
    const words = (m: Record<string, unknown>, live = false) => callLineWords(callLineOf(line(m))!, tr, live);
    expect(words({ direction: 'out', outcome: 'ended', duration_secs: 151, via: 'direct' })).toEqual({ title: 'msg_call_line_out', detail: '2:31' });
    expect(words({ direction: 'in', outcome: 'ended', duration_secs: 9, via: 'relay' })).toEqual({ title: 'msg_call_line_in', detail: '0:09 · msg_call_detail_relay' });
    expect(words({ direction: 'out', outcome: 'missed' })).toEqual({ title: 'msg_call_line_out', detail: 'msg_call_detail_no_answer' });
    expect(words({ direction: 'in', outcome: 'declined' })).toEqual({ title: 'msg_call_line_in', detail: 'msg_call_detail_declined' });
    expect(words({ direction: 'out', outcome: 'busy', media: 'video' })).toEqual({ title: 'msg_call_line_out_video', detail: 'msg_call_detail_busy' });
    expect(words({ direction: 'in', outcome: null })).toEqual({ title: 'msg_call_line_in', detail: null });
    expect(words({ direction: 'in', outcome: null }, true)).toEqual({ title: 'msg_call_line_in', detail: 'msg_call_detail_now' });
  });

  it('makes a call of the peer that I did not take stand out', () => {
    const of = (m: Record<string, unknown>) => callLineOf(line(m))!;
    expect(isMissed(of({ direction: 'in', outcome: 'missed' }))).toBe(true);
    expect(isMissed(of({ direction: 'in', outcome: 'busy' }))).toBe(true);
    expect(isMissed(of({ direction: 'out', outcome: 'missed' }))).toBe(false);
    expect(isMissed(of({ direction: 'in', outcome: 'declined' }))).toBe(false);
    expect(callLineWords(of({ direction: 'in', outcome: 'missed', media: 'video' }), tr).title).toBe('msg_call_line_missed_video');
  });
});

describe('a refusal of a call', () => {
  it('is worded when the runtime\'s sentence is known, and passed on when not', () => {
    expect(callErrorText({ code: 'other', message: 'calls go to contacts only' }, tr)).toBe('msg_call_err_contacts');
    expect(callErrorText({ code: 'other', message: 'a call is under way' }, tr)).toBe('msg_call_err_busy');
    expect(callErrorText({ code: 'transport', message: 'this build has no media engine (feature rtc)' }, tr)).toBe('msg_call_unavailable');
    expect(callErrorText('node unreachable', tr)).toBe('msg_call_err_other(node unreachable)');
    expect(callErrorText({ code: 'other', message: 'no camera on this machine' }, tr)).toBe('msg_call_err_no_camera');
    expect(callErrorText('/dev/video0: no uncompressed format (YUYV or NV12); it offers MJPG', tr)).toMatch(/^msg_call_err_camera\(/);
    expect(callErrorText('the screen cannot be captured here (no display)', tr)).toMatch(/^msg_call_err_screen\(/);
  });

  it('words the refusals of a group call', () => {
    expect(callErrorText({ code: 'invalid', message: 'a call is on in this group already: join it' }, tr)).toBe('msg_gcall_err_on');
    expect(callErrorText({ code: 'invalid', message: 'no call is on in this group' }, tr)).toBe('msg_gcall_err_gone');
    expect(callErrorText({ code: 'transport', message: 'POST /v1/rooms/ab/join: 404 room_not_found' }, tr)).toBe('msg_gcall_err_gone');
    expect(callErrorText({ code: 'transport', message: 'join: 409 room_full' }, tr)).toBe('msg_gcall_err_full');
    expect(callErrorText({ code: 'invalid', message: 'a group call is under way' }, tr)).toBe('msg_call_err_busy');
    expect(callErrorText({ code: 'transport', message: 'no call node with an SFU answered' }, tr)).toBe('msg_gcall_err_no_node');
    expect(callErrorText({ code: 'transport', message: 'the way to the node is lost' }, tr)).toBe('msg_gcall_err_node_lost');
  });
});
