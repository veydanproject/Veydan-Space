// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What each place that offers «Report» hands the command, and what the
// person is told when it answers.

import { beforeEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({ send: vi.fn() }));
vi.mock('../api', () => ({ messengerApi: { report: { send: api.send } } }));

const toast = vi.hoisted(() => ({ success: vi.fn(), info: vi.fn(), error: vi.fn() }));
vi.mock('$lib/core/ui/toast.svelte', () => ({ toast }));

vi.mock('$lib/core/i18n', async () => {
  const { readable } = await import('svelte/store');
  // The key and its parameters, so that a test sees which words were chosen.
  return { t: readable((key: string, p?: Record<string, string>) => (p ? `${key} ${JSON.stringify(p)}` : key)) };
});

const { reportStore } = await import('./report.svelte');

const sent = { status: 'sent', attached: 3, proved: 3, blocked: true } as const;

beforeEach(() => {
  api.send.mockReset();
  toast.success.mockReset(); toast.info.mockReset(); toast.error.mockReset();
  api.send.mockResolvedValue(sent);
});

describe('the report each entry builds', () => {
  it('a message of a chat of two: attached up to it, and the person blocked', async () => {
    reportStore.open({ accused: 'aa', name: 'Ann', context: { t: 'dm' }, messageId: 'm1', canBlock: true });
    reportStore.reason = 'spam';
    await reportStore.send();
    expect(api.send).toHaveBeenCalledWith({
      accused: 'aa', context: { t: 'dm' }, reason: 'spam', sub_reason: null, comment: null,
      attach: true, message_id: 'm1', block: true,
    });
  });

  it('a message of a group: the group named, nobody blocked', async () => {
    reportStore.open({ accused: 'bb', name: 'Bob', context: { t: 'group', group_id: 'g1' }, messageId: 'm2', canBlock: false });
    reportStore.reason = 'harassment';
    reportStore.block = true; // a leftover must not travel
    await reportStore.send();
    expect(api.send).toHaveBeenCalledWith(expect.objectContaining({ context: { t: 'group', group_id: 'g1' }, attach: true, message_id: 'm2', block: false }));
  });

  it('a request: the person only, nothing attached by default, blockable', async () => {
    reportStore.open({ accused: 'cc', name: 'Cy', context: { t: 'request' }, canBlock: true });
    reportStore.reason = 'illegal';
    await reportStore.send();
    expect(api.send).toHaveBeenCalledWith(expect.objectContaining({ context: { t: 'request' }, attach: false, message_id: null, block: true }));
  });

  it('a post of a channel: the channel and the post named, nothing of a chat attached or blocked', async () => {
    const context = { t: 'channel' as const, community_id: 'c'.repeat(64), post_id: 'd'.repeat(64) };
    reportStore.open({ accused: 'dd', name: 'Dee', context, messageId: 'm3', canBlock: true });
    reportStore.reason = 'spam';
    reportStore.block = true;
    await reportStore.send();
    expect(api.send).toHaveBeenCalledWith(expect.objectContaining({ accused: 'dd', context, attach: false, message_id: null, block: false }));
  });

  it('a harm to a child carries its kind, and only then; the comment is trimmed', async () => {
    reportStore.open({ accused: 'aa', name: 'Ann', context: { t: 'dm' }, canBlock: true });
    reportStore.reason = 'child_harm';
    reportStore.sub = 'grooming';
    reportStore.comment = '  see above  ';
    expect(reportStore.input()).toMatchObject({ reason: 'child_harm', sub_reason: 'grooming', comment: 'see above' });
    reportStore.reason = 'other';
    expect(reportStore.input()?.sub_reason).toBeNull();
  });

  it('nothing goes before a reason is chosen', async () => {
    reportStore.open({ accused: 'aa', name: 'Ann', context: { t: 'dm' }, canBlock: true });
    expect(reportStore.ready).toBe(false);
    await reportStore.send();
    expect(api.send).not.toHaveBeenCalled();
  });
});

describe('what the person is told', () => {
  const start = (messageId: string | null = 'm1') => {
    reportStore.open({ accused: 'aa', name: 'Ann', context: { t: 'dm' }, messageId, canBlock: true });
    reportStore.reason = 'spam';
  };

  it('sent with messages attached', async () => {
    start();
    await reportStore.send();
    expect(toast.success).toHaveBeenCalledWith('msg_report_sent_n {"n":"3"}');
    expect(reportStore.target).toBeNull();
  });

  it('sent with none attached', async () => {
    start(null);
    api.send.mockResolvedValue({ ...sent, attached: 0 });
    await reportStore.send();
    expect(toast.success).toHaveBeenCalledWith('msg_report_sent');
  });

  it('queued while there is no connection', async () => {
    start();
    api.send.mockResolvedValue({ ...sent, status: 'queued' });
    await reportStore.send();
    expect(toast.info).toHaveBeenCalledWith('msg_report_queued');
  });

  it('too many from this device; the form stays with what was written', async () => {
    start();
    reportStore.comment = 'text';
    api.send.mockRejectedValue('rate_limited');
    await reportStore.send();
    expect(toast.error).toHaveBeenCalledWith('msg_report_rate_limited');
    expect(reportStore.target).not.toBeNull();
    expect(reportStore.comment).toBe('text');
    expect(reportStore.busy).toBe(false);
  });

  it('any other failure', async () => {
    start();
    api.send.mockRejectedValue(new Error('boom'));
    await reportStore.send();
    expect(toast.error).toHaveBeenCalledWith('msg_report_failed');
  });

  it('tells the entry what became of the person', async () => {
    const onsent = vi.fn();
    reportStore.open({ accused: 'aa', name: 'Ann', context: { t: 'request' }, canBlock: true, onsent });
    reportStore.reason = 'spam';
    await reportStore.send();
    expect(onsent).toHaveBeenCalledWith(sent);
  });
});

describe('a report on a whole group', () => {
  it('names the group, sends no accused and never blocks', async () => {
    reportStore.open({ target: 'group', accused: '', name: 'Club', context: { t: 'group', group_id: 'g9' }, messageId: 'm7', canBlock: false });
    reportStore.reason = 'child_harm';
    reportStore.sub = 'sexual';
    reportStore.block = true; // a leftover must not travel
    await reportStore.send();
    expect(api.send).toHaveBeenCalledWith({
      target: 'group', accused: '', context: { t: 'group', group_id: 'g9' }, reason: 'child_harm', sub_reason: 'sexual', comment: null,
      attach: true, message_id: 'm7', block: false,
    });
  });

  it('a person report does not carry a target', () => {
    reportStore.open({ accused: 'aa', name: 'Ann', context: { t: 'dm' }, canBlock: true });
    reportStore.reason = 'spam';
    expect(reportStore.input()).not.toHaveProperty('target');
  });
});

describe('a group the network closed', () => {
  it('is told by the chat row', async () => {
    const { isBannedGroup } = await import('../groups/permissions');
    expect(isBannedGroup({ banned: true })).toBe(true);
    expect(isBannedGroup({})).toBe(false);
    expect(isBannedGroup(null)).toBe(false);
  });
});
