// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// On localhost the login is over before `ssh_connect` returns: the server's key
// question and the status of the new session reach the store before the
// terminal is drawn, and are kept for it (ROO-822).

import { describe, expect, it, vi } from 'vitest';

type Handler = (e: { payload: unknown }) => void;
const handlers = vi.hoisted(() => new Map<string, Handler>());
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, cb: Handler) => {
    handlers.set(name, cb);
    return () => handlers.delete(name);
  }),
}));

const emit = (name: string, payload: unknown) => handlers.get(name)?.({ payload });

const api = vi.hoisted(() => ({
  ssh: {
    connect: vi.fn(async (): Promise<string> => ''),
    sessionRemove: vi.fn(async () => {}),
    connectionGet: vi.fn(async () => ({ id: 'c1', name: 'qa', host: '127.0.0.1', port: 2222 })),
  },
}));
vi.mock('../api', () => ({ api }));

vi.stubGlobal('window', { __TAURI_INTERNALS__: {} });
const { sshStore } = await import('./ssh.svelte');

describe('a login that ends before connect returns', () => {
  it('keeps the host key question for the terminal, once', async () => {
    api.ssh.connect.mockImplementationOnce(async () => {
      expect(handlers.has('ssh://keyboard-prompt')).toBe(true);
      emit('ssh://keyboard-prompt', {
        session_id: 's1',
        name: 'Host key verification',
        instructions: '127.0.0.1:2222 SHA256:abc',
        prompts: [{ prompt: 'Trust this host? (yes/no)', echo: true }],
        host_key: 'SHA256:abc',
      });
      return 's1';
    });
    const id = await sshStore.connect('c1');
    expect(sshStore.prompts[id]?.host_key).toBe('SHA256:abc');
    expect(sshStore.takePrompt(id)?.host_key).toBe('SHA256:abc');
    expect(sshStore.takePrompt(id)).toBeUndefined();
  });

  it('keeps the status the session reached', async () => {
    api.ssh.connect.mockImplementationOnce(async () => {
      emit('ssh://status-changed', { session_id: 's2', connection_id: 'c1', status: 'connected' });
      return 's2';
    });
    await sshStore.connect('c1');
    expect(sshStore.sessionById('s2')?.status).toBe('connected');
  });

  it.each([
    ['s6', 'c9', 'error', 'Permission denied'],
    ['s7', 'c8', 'connected', null],
  ])('keeps a status that comes while the connection is fetched (%s, %s, %s)', async (sid, cid, status, error) => {
    api.ssh.connect.mockImplementationOnce(async () => sid);
    api.ssh.connectionGet.mockImplementationOnce(async () => {
      await Promise.resolve();
      emit('ssh://status-changed', { session_id: sid, connection_id: cid, status, error: error ?? undefined });
      return { id: cid, name: 'other', host: '127.0.0.1', port: 2222 };
    });
    await sshStore.connect(cid);
    expect(sshStore.sessionById(sid)?.status).toBe(status);
    expect(sshStore.sessionById(sid)?.error).toBe(error);
  });

  it('leaves the questions of SFTP to the files store', () => {
    emit('ssh://keyboard-prompt', { session_id: 'sftp:c1', name: '', instructions: '', prompts: [] });
    expect(sshStore.prompts['sftp:c1']).toBeUndefined();
  });
});

describe('statuses no connect waits for', () => {
  const held = () => (sshStore as unknown as { earlyStatus: Map<string, unknown> }).earlyStatus;

  it('are let go when connect fails', async () => {
    api.ssh.connect.mockImplementationOnce(async () => {
      emit('ssh://status-changed', { session_id: 's3', connection_id: 'c1', status: 'connecting' });
      throw new Error('Host key verification failed');
    });
    await expect(sshStore.connect('c1')).rejects.toThrow();
    expect(held().size).toBe(0);
  });

  it('are not held after the session is removed', async () => {
    api.ssh.connect.mockImplementationOnce(async () => 's4');
    await sshStore.connect('c1');
    await sshStore.removeSession('s4');
    emit('ssh://status-changed', { session_id: 's4', connection_id: 'c1', status: 'disconnected' });
    expect(held().size).toBe(0);
    expect(sshStore.sessionById('s4')).toBeUndefined();
  });

  it('let the question of the login go when connect fails after it', async () => {
    api.ssh.connect.mockImplementationOnce(async () => {
      emit('ssh://keyboard-prompt', { session_id: 's8', name: '', instructions: '', prompts: [{ prompt: 'Password:', echo: false }] });
      throw new Error('Authentication timeout (120s)');
    });
    await expect(sshStore.connect('c1')).rejects.toThrow();
    expect(sshStore.prompts['s8']).toBeUndefined();
  });

  it('let the question of the login go when the session ends before the terminal takes it', async () => {
    api.ssh.connect.mockImplementationOnce(async () => 's9');
    await sshStore.connect('c1');
    emit('ssh://keyboard-prompt', { session_id: 's9', name: '', instructions: '', prompts: [{ prompt: 'Password:', echo: false }] });
    expect(sshStore.prompts['s9']).toBeDefined();
    emit('ssh://status-changed', { session_id: 's9', connection_id: 'c1', status: 'error', error: 'Disconnected during authentication' });
    expect(sshStore.prompts['s9']).toBeUndefined();
  });

  it('do not hold a question of a session no connect waits for', () => {
    emit('ssh://keyboard-prompt', { session_id: 'x2', name: '', instructions: '', prompts: [] });
    expect(sshStore.prompts['x2']).toBeUndefined();
  });

  it('of a session of another connection are not held', async () => {
    api.ssh.connect.mockImplementationOnce(async () => {
      emit('ssh://status-changed', { session_id: 'x1', connection_id: 'c2', status: 'connected' });
      return 's5';
    });
    await sshStore.connect('c1');
    expect(held().size).toBe(0);
  });
});
