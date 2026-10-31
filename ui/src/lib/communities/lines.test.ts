// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from ui/src/lib/messenger/groups/lines.test.ts at 8ce09c39, then changed for channels.

import { describe, expect, it } from 'vitest';
import type { ChannelDetail, ChannelView } from './api';
import { joinLine, previewLine, rulesLines } from './lines';

// A translation that shows the key and the words it was given.
const tr = (key: string, params?: Record<string, string>) => (params ? `${key}(${Object.values(params).join(',')})` : key);
const days = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];

const view = (extra: Partial<ChannelView> = {}): ChannelView => ({
  id: 'c', chat_id: 'channel:c', name: 'News', about: '', owner: 'o', join: 'open', membership: 'joined', members: 3, relays: [], storage: [],
  link_epoch: 1, kept_here: false, needs_update: false, unread: 0, pinned: false, muted: false, archived: false, created_at: 0, updated_at: 0, last_at: 0, ...extra,
});
const detail = (extra: Partial<ChannelDetail> = {}, v: Partial<ChannelView> = {}): ChannelDetail => ({
  channel: view(v), history: 'open', post_mode: 'authors', comments: 'on', premod: 'off', pinned: [], staff: [], disbanded: false,
  has_key: true, waiting: 0, waiting_keeper: 0, keeper_refused: false, ...extra,
});

describe('the line of a channel in the list of chats', () => {
  it('is the first line of the newest post', () => {
    expect(previewLine(view({ last: { id: 'p', author: 'a', text: 'Hello\nworld', has_media: false, at: 1 } }), tr)).toBe('Hello');
  });

  it('is «a file» for a file without words, «no posts» for an empty channel', () => {
    expect(previewLine(view({ last: { id: 'p', author: 'a', has_media: true, at: 1 } }), tr)).toBe('chan_post_file');
    expect(previewLine(view({ last: { id: 'p', author: 'a', has_media: true, media_kind: 'voice', at: 1 } }), tr)).toBe('msg_notice_voice');
    expect(previewLine(view({ last: { id: 'p', author: 'a', has_media: true, media_kind: 'circle', at: 1 } }), tr)).toBe('msg_notice_circle');
    expect(previewLine(view(), tr)).toBe('chan_no_posts');
  });

  it('says where I stand when I am not in', () => {
    expect(previewLine(view({ membership: 'joining' }), tr)).toBe('chan_state_joining');
    expect(previewLine(view({ membership: 'requested' }), tr)).toBe('chan_state_requested');
    expect(previewLine(view({ membership: 'out' }), tr)).toBe('chan_state_out');
    // Why I am out (ревью UX п. 8); a reason this build does not know is plain «out».
    expect(previewLine(view({ membership: 'out', out_reason: 'banned' }), tr)).toBe('chan_state_out_banned');
    expect(previewLine(view({ membership: 'out', out_reason: 'later' }), tr)).toBe('chan_state_out');
    expect(previewLine(view({ membership: 'disbanded', out_reason: 'disbanded' }), tr)).toBe('chan_state_disbanded');
  });
});

describe('the rules on the card of a channel', () => {
  it('names the way in and the history, a word of a newer core as it came', () => {
    expect(joinLine(detail(), tr)).toBe('chan_join_open · chan_history_open');
    expect(joinLine(detail({ history: 'later' }, { join: 'request' }), tr)).toBe('chan_join_request · later');
  });

  it('leaves out the check and the hours while they are off', () => {
    expect(rulesLines(detail(), tr, days)).toEqual([
      'chan_join_open · chan_history_open', 'chan_rule_comments: chan_on',
    ]);
  });

  it('words the check and the hours with their zone', () => {
    const lines = rulesLines(detail({ premod: 'newcomers', schedule: { tz: 'Europe/Berlin', days: [1, 2, 3, 4, 5], from: '09:00', to: '18:00' } }), tr, days);
    expect(lines.slice(2)).toEqual(['chan_rule_premod: chan_premod_newcomers', 'chan_rule_schedule: Mon–Fri 09:00–18:00 (Europe/Berlin)']);
  });
});
