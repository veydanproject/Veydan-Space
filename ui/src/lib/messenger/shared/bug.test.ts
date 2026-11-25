// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// When a bug report can go to the project, what the command is handed, and
// what is said of a report that waited.

import { beforeEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({ send: vi.fn(), diagnose: vi.fn() }));
vi.mock('../api', () => ({ messengerApi: { bug: { send: api.send }, net: { diagnose: api.diagnose } } }));

const store = vi.hoisted(() => ({
  ensureLoaded: vi.fn(async () => {}),
  status: { runtime: {} } as { runtime: object | null } | null,
  identity: { npub: 'npub1' } as object | null,
  pendingBackup: false,
  serversMode: 'veydan' as 'veydan' | 'own' | null,
}));
vi.mock('../store.svelte', () => ({ messengerStore: store }));

const toast = vi.hoisted(() => ({ success: vi.fn(), info: vi.fn(), error: vi.fn() }));
vi.mock('$lib/core/ui/toast.svelte', () => ({ toast }));

vi.mock('$lib/core/i18n', async () => {
  const { readable } = await import('svelte/store');
  return { t: readable((key: string) => key) };
});

const { bugSender } = await import('./bug');
const { bugEvent } = await import('./bugEvent');

beforeEach(() => {
  api.send.mockReset(); api.diagnose.mockReset();
  toast.success.mockReset(); toast.info.mockReset(); toast.error.mockReset();
  store.status = { runtime: {} };
  store.identity = { npub: 'npub1' };
  store.pendingBackup = false;
  store.serversMode = 'veydan';
});

describe('whether a bug report can be sent', () => {
  it('with the account whole on the servers of the project', async () => {
    expect(await bugSender.available()).toBe(true);
  });

  it('not with the user’s own servers: nothing goes to the project', async () => {
    store.serversMode = 'own';
    expect(await bugSender.available()).toBe(false);
  });

  it('not without an account, a copy of the key owed, or the messenger stopped', async () => {
    store.identity = null;
    expect(await bugSender.available()).toBe(false);
    store.identity = { npub: 'npub1' };
    store.pendingBackup = true;
    expect(await bugSender.available()).toBe(false);
    store.pendingBackup = false;
    store.status = { runtime: null };
    expect(await bugSender.available()).toBe(false);
    store.status = { runtime: {} };
    store.serversMode = null;
    expect(await bugSender.available()).toBe(false);
  });
});

describe('the report sent', () => {
  const report = { text: 'It broke', product: 'space', version: '5.2.0', os: 'linux/x86_64', locale: 'ru', modules: ['Camoufox: 135'] };

  it('goes as it was handed, the diagnosis with it, and answers how it went', async () => {
    api.send.mockResolvedValue({ status: 'sent', id: 'abc' });
    const net = { v: 1 };
    expect(await bugSender.send({ ...report, net })).toBe('sent');
    expect(api.send).toHaveBeenCalledWith({ ...report, net });
    api.send.mockResolvedValue({ status: 'queued', id: null });
    expect(await bugSender.send(report)).toBe('queued');
  });

  it('a refusal is the caller’s to word', async () => {
    api.send.mockRejectedValue('rate_limited');
    await expect(bugSender.send(report)).rejects.toBe('rate_limited');
  });
});

describe('a report that waited', () => {
  it('is said once it went, was refused or was given up', () => {
    bugEvent({ name: 'bug.report', payload: { status: 'sent', id: 'abc' } });
    expect(toast.success).toHaveBeenCalledWith('msg_bug_sent_later');
    bugEvent({ name: 'bug.report', payload: { status: 'refused', error: 'bug_text' } });
    expect(toast.error).toHaveBeenCalledWith('msg_bug_refused');
    bugEvent({ name: 'bug.report', payload: { status: 'expired' } });
    expect(toast.error).toHaveBeenCalledWith('msg_bug_expired');
  });

  it('another event says nothing', () => {
    bugEvent({ name: 'moderation.report', payload: { status: 'sent' } });
    expect(toast.success).not.toHaveBeenCalled();
    expect(toast.error).not.toHaveBeenCalled();
  });
});
