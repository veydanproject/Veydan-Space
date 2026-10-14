// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SaveQueue, type SaveStatus } from './save-queue';

/** An editor buffer, a disk and a write the test lets through by hand. */
function harness(opts: { empty?: boolean } = {}) {
  const s = {
    buffer: '',
    knownBody: '',
    disk: '',
    held: false,
    status: 'idle' as SaveStatus,
    writes: [] as string[],
    failures: 0,
    gates: [] as { resolve: () => void; reject: (e: unknown) => void }[],
  };
  const queue = new SaveQueue({
    write: async () => {
      const sent = s.buffer;
      s.writes.push(sent);
      if (opts.empty && !sent) return false;
      await new Promise<void>((resolve, reject) => s.gates.push({ resolve, reject }));
      s.disk = sent;
      s.knownBody = sent;
      return true;
    },
    held: () => s.held,
    failed: (e) => {
      s.failures++;
      if (e === 'conflict') {
        s.held = true;
        return true;
      }
      return false;
    },
    status: (x) => (s.status = x),
  });
  const type = (text: string) => {
    s.buffer += text;
    queue.edit();
  };
  const pass = async () => {
    s.gates.shift()!.resolve();
    await vi.advanceTimersByTimeAsync(0);
  };
  const fail = async (e: unknown = new Error('disk full')) => {
    s.gates.shift()!.reject(e);
    await vi.advanceTimersByTimeAsync(0);
  };
  return { s, queue, type, pass, fail };
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('SaveQueue', () => {
  it('an edit typed while the timer write is in flight is written next, not marked saved', async () => {
    const { s, queue, type, pass } = harness();
    type('a');
    await vi.advanceTimersByTimeAsync(700);
    expect(s.writes).toEqual(['a']);
    expect(s.status).toBe('saving');

    type('b');
    await pass();
    // The disk holds "a", the screen "ab": the badge must not say saved.
    expect(s.status).toBe('dirty');
    expect(queue.unsaved).toBe(true);

    await vi.advanceTimersByTimeAsync(700);
    expect(s.writes).toEqual(['a', 'ab']);
    await pass();
    expect(s.disk).toBe('ab');
    expect(s.status).toBe('saved');
    expect(queue.unsaved).toBe(false);
  });

  it('an explicit save waits for the write in flight and writes what was typed during it', async () => {
    const { s, queue, type, pass } = harness();
    type('a');
    const done = queue.save();
    type('b');
    await pass();
    // No new timer is needed: the same save goes on with the newer buffer.
    expect(s.writes).toEqual(['a', 'ab']);
    await pass();
    await done;
    expect(s.disk).toBe('ab');
    expect(s.status).toBe('saved');
    await vi.advanceTimersByTimeAsync(700);
    expect(s.writes).toEqual(['a', 'ab']);
  });

  it('a failed write keeps the buffer unsaved, and the next save tries again', async () => {
    const { s, queue, type, pass, fail } = harness();
    type('hello');
    const first = queue.save();
    await fail();
    await first;
    expect(s.status).toBe('failed');
    expect(s.failures).toBe(1);
    expect(s.buffer).toBe('hello');
    expect(s.disk).toBe('');
    expect(queue.unsaved).toBe(true);

    // Leaving the page: the badge says failed, the buffer still differs from the disk.
    expect(s.buffer !== s.knownBody).toBe(true);
    const leave = queue.save();
    await pass();
    await leave;
    expect(s.disk).toBe('hello');
    expect(s.status).toBe('saved');
    expect(queue.unsaved).toBe(false);
  });

  it('leaving with nothing unsaved writes nothing', async () => {
    const { s, queue, type, pass } = harness();
    type('x');
    const p = queue.save();
    await pass();
    await p;
    expect(s.buffer === s.knownBody).toBe(true);
    await queue.save();
    expect(s.writes).toEqual(['x']);
  });

  it('a conflict holds writes until the user decides', async () => {
    const { s, queue, type, fail } = harness();
    type('mine');
    const p = queue.save();
    await fail('conflict');
    await p;
    expect(s.held).toBe(true);
    expect(s.status).toBe('dirty');
    type(' more');
    await vi.advanceTimersByTimeAsync(5000);
    await queue.save();
    expect(s.writes).toEqual(['mine']);
    expect(queue.unsaved).toBe(true);
  });

  it('discard drops the pending edits and their timer', async () => {
    const { s, queue, type } = harness();
    type('gone');
    queue.discard();
    await vi.advanceTimersByTimeAsync(5000);
    await queue.save();
    expect(s.writes).toEqual([]);
    expect(queue.unsaved).toBe(false);
  });

  it('an empty new note is not created and the badge goes idle', async () => {
    const { s, queue } = harness({ empty: true });
    queue.edit();
    await queue.save();
    expect(s.writes).toEqual(['']);
    expect(s.status).toBe('idle');
    expect(queue.unsaved).toBe(false);
  });
});
