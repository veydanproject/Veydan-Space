// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { MessengerMessage } from '../api';
import { groupLine, type Words } from './lines';

// A translation that shows the key and the words it was given, in the order they are named.
const tr = (key: string, params?: Record<string, string>) =>
  params ? `${key}(${Object.entries(params).filter(([, v]) => v).map(([k, v]) => `${k}=${v}`).join(',')})` : key;
const name = (pk: string) => `n:${pk}`;
const words: Words = { stamp: (unix) => `t${unix}`, days: () => ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'] };

const line = (text: string, media: object = {}, content_type = 'system') => ({ content_type, text, media }) as unknown as MessengerMessage;
const say = (text: string, media: object = {}) => groupLine(line(text, { actor: 'a', ...media }), tr, name, words);

describe('groupLine', () => {
  it('leaves alone what is not a line of a group', () => {
    expect(groupLine(line('group_created', {}, 'text'), tr, name, words)).toBeNull();
    expect(groupLine(line('request_sent'), tr, name, words)).toBeNull();
  });

  it('words the old kinds by their actor, target and role', () => {
    expect(say('group_role', { target: 'b', role: 'admin' })).toBe('msg_sys_group_role(actor=n:a,target=n:b,role=msg_group_role_admin)');
  });

  it('words a mute for a time with the local time of its end', () => {
    expect(say('group_muted_until', { target: 'b', until: 3600 })).toBe('msg_sys_group_muted_until(actor=n:a,target=n:b,time=t3600)');
  });

  it('words the trust given and taken back', () => {
    expect(say('group_trusted', { target: 'b' })).toBe('msg_sys_group_trusted(actor=n:a,target=n:b)');
    expect(say('group_untrusted', { target: 'b' })).toBe('msg_sys_group_untrusted(actor=n:a,target=n:b)');
  });

  it('words who may write by the words of the settings', () => {
    expect(say('group_post_mode', { post_mode: 'managers' })).toBe('msg_sys_group_post_mode(actor=n:a,mode=msg_mod_post_mode_managers)');
    expect(say('group_post_mode', { post_mode: 'all' })).toBe('msg_sys_group_post_mode(actor=n:a,mode=msg_mod_post_mode_all)');
    expect(say('group_post_mode', { post_mode: 'later' })).toBe('msg_sys_group_post_mode(actor=n:a,mode=later)');
  });

  it('words the check of messages by the words of the settings', () => {
    for (const p of ['off', 'newcomers', 'all']) {
      expect(say('group_premod', { premod: p })).toBe(`msg_sys_group_premod(actor=n:a,mode=msg_mod_premod_${p})`);
    }
  });

  it('words a schedule with its zone, and its removal', () => {
    const s = { tz: 'Europe/Berlin', days: [1, 2, 3, 4, 5], from: '22:00', to: '08:00' };
    expect(say('group_schedule', { schedule: s })).toBe('msg_sys_group_schedule(actor=n:a,schedule=Mon–Fri 22:00–08:00 (Europe/Berlin))');
    expect(say('group_schedule', { schedule: { ...s, days: [1, 2, 3, 4, 5, 6, 7] } })).toBe(
      'msg_sys_group_schedule(actor=n:a,schedule=22:00–08:00 (Europe/Berlin))',
    );
    expect(say('group_schedule', { schedule: null })).toBe('msg_sys_group_schedule(actor=n:a,schedule=msg_mod_schedule_off)');
  });
});
