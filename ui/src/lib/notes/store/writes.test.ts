// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A note opened again is read after the writes of it begun before, whatever their end.

import { describe, expect, it } from 'vitest';
import { trackWrite, writesSettled } from './writes';

function gate() {
  let resolve!: () => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<void>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe('writesSettled', () => {
  it('waits for every write of the note begun before it, and for no other note', async () => {
    const first = gate();
    const second = gate();
    void trackWrite('a', first.promise);
    void trackWrite('a', second.promise).catch(() => {});
    void trackWrite('b', new Promise(() => {}));

    let read = false;
    void writesSettled('a').then(() => (read = true));
    first.resolve();
    await flush();
    expect(read).toBe(false);

    second.reject(new Error('refused'));
    await flush();
    expect(read).toBe(true);
  });

  it('does not wait when nothing is being written', async () => {
    let read = false;
    void writesSettled('c').then(() => (read = true));
    await flush();
    expect(read).toBe(true);
  });
});
