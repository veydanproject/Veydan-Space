// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A file of mine on its way shows the line of a chat's transfer: its
// progress as told, or made of its row before it moves; each action of the
// line goes to the store.

import { describe, expect, it, vi } from 'vitest';
import { uploadAction, uploadLabel, uploadProgress, uploadsUnder } from './uploads';

const row = { id: 'local:c1', communityId: 'c', name: 'film.mp4', status: 'paused', done: 4, total: 9 };

describe('a file of mine on its way', () => {
  it('is told as its row says before it moves', () => {
    expect(uploadProgress(row)).toMatchObject({ transfer_id: 'local:c1', direction: 'up', status: 'paused', done_bytes: 4, total_bytes: 9, file_name: 'film.mp4' });
    expect(uploadProgress({ ...row, status: 'uploading' }).status).toBe('queued');
  });

  it('is told as the runtime tells it once it moves', () => {
    const told = { ...uploadProgress(row), status: 'running' as const, rate_bps: 1000 };
    expect(uploadProgress({ ...row, progress: told })).toBe(told);
  });

  it('is named by its kind when it is a recording, by its name otherwise', () => {
    const title = (kind: string) => (kind === 'voice' ? 'Голосовое сообщение' : 'Видеосообщение');
    expect(uploadLabel({ name: 'voice.weba', kind: 'voice' }, title, '…')).toBe('Голосовое сообщение');
    expect(uploadLabel({ name: 'circle.webm', kind: 'circle' }, title, '…')).toBe('Видеосообщение');
    expect(uploadLabel({ name: 'voice.weba' }, title, '…')).toBe('voice.weba');
    expect(uploadLabel({ name: '' }, title, 'Загрузка')).toBe('Загрузка');
  });

  it('is shown under its post when it is a comment, at the foot of the feed when it is a post', () => {
    const all = [{ id: 'a', communityId: 'c', postId: '' }, { id: 'b', communityId: 'c', postId: 'p1' }, { id: 'd', communityId: 'x', postId: 'p1' }, { id: 'e', communityId: 'c' }];
    expect(uploadsUnder(all, 'c', 'p1').map((u) => u.id)).toEqual(['b']);
    expect(uploadsUnder(all, 'c', '').map((u) => u.id)).toEqual(['a', 'e']);
  });

  it('pauses, wakes, cancels, and goes on from what the hosts hold on a resume or a retry', async () => {
    const store = { pauseUpload: vi.fn(async () => {}), wakeUpload: vi.fn(async () => {}), retryUpload: vi.fn(async () => {}), cancelUpload: vi.fn(async () => {}) };
    for (const a of ['pause', 'retry_now', 'cancel', 'resume', 'retry'] as const) await uploadAction(store, 'u', a);
    expect([store.pauseUpload, store.wakeUpload, store.cancelUpload].map((f) => f.mock.calls.length)).toEqual([1, 1, 1]);
    expect(store.retryUpload).toHaveBeenCalledTimes(2);
  });
});
