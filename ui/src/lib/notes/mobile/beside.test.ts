// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { emptyTrashClosing, reloadOnWrites } from './beside';

describe('the list beside a note (core/ui/split)', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  function writes() {
    const cbs = new Set<() => void>();
    return {
      subscribe: (cb: () => void) => (cbs.add(cb), () => cbs.delete(cb)),
      write: () => cbs.forEach((cb) => cb()),
    };
  }

  it('reloads after a write landing once the editor is left (Enter, then back at once)', () => {
    const w = writes();
    const load = vi.fn();
    reloadOnWrites(w.subscribe, load, () => false);
    w.write();
    w.write();
    expect(load).not.toHaveBeenCalled();
    vi.advanceTimersByTime(300);
    expect(load).toHaveBeenCalledTimes(1);
  });

  it('leaves to the list its own actions, and stops with it', () => {
    const w = writes();
    const load = vi.fn();
    let busy = true;
    const stop = reloadOnWrites(w.subscribe, load, () => busy);
    w.write();
    vi.advanceTimersByTime(300);
    expect(load).not.toHaveBeenCalled();
    busy = false;
    w.write();
    stop();
    vi.advanceTimersByTime(300);
    w.write();
    vi.advanceTimersByTime(300);
    expect(load).not.toHaveBeenCalled();
  });

  it('closes a note of the trash beside the list once the trash is emptied, a search hiding it or not', async () => {
    const order: string[] = [];
    const notes = {
      list: vi.fn(async (search: string) => {
        order.push(`list:${search}`);
        return [{ id: 'a' }, { id: 'b' }];
      }),
      emptyTrash: vi.fn(async () => order.push('empty')),
    };
    const close = vi.fn((ids: string[]) => order.push(`close:${ids.join(',')}`));
    await emptyTrashClosing(notes, close);
    expect(order).toEqual(['list:', 'empty', 'close:a,b']);
  });

  it('closes nothing when the trash was not emptied', async () => {
    const notes = {
      list: async () => [{ id: 'a' }],
      emptyTrash: async () => {
        throw new Error('no');
      },
    };
    const close = vi.fn();
    await expect(emptyTrashClosing(notes, close)).rejects.toThrow('no');
    expect(close).not.toHaveBeenCalled();
  });
});
