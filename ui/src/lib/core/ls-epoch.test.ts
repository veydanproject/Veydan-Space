// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { applyLsEpoch, LS_EPOCH, LS_EPOCH_KEY } from './ls-epoch';

class MemoryStorage implements Storage {
  private items = new Map<string, string>();
  clears = 0;

  constructor(initial: Record<string, string> = {}) {
    for (const [k, v] of Object.entries(initial)) this.items.set(k, v);
  }

  get length(): number {
    return this.items.size;
  }
  key(index: number): string | null {
    return [...this.items.keys()][index] ?? null;
  }
  getItem(key: string): string | null {
    return this.items.get(key) ?? null;
  }
  setItem(key: string, value: string): void {
    this.items.set(key, value);
  }
  removeItem(key: string): void {
    this.items.delete(key);
  }
  clear(): void {
    this.clears += 1;
    this.items.clear();
  }
  snapshot(): Record<string, string> {
    return Object.fromEntries(this.items);
  }
}

describe('ls-epoch', () => {
  it('uses the key and value of the platform spec', () => {
    expect(LS_EPOCH_KEY).toBe('veydan_ls_epoch');
    expect(LS_EPOCH).toBe('5');
  });

  it('drops everything a 4.x build left behind', () => {
    const storage = new MemoryStorage({ m_default_app: 'passwords', vb_locale: 'ru', rb_theme: 'light' });
    applyLsEpoch(storage);
    expect(storage.snapshot()).toEqual({ veydan_ls_epoch: '5' });
  });

  it('drops keys of any other epoch value', () => {
    for (const stale of ['4', '6', '05', '']) {
      const storage = new MemoryStorage({ veydan_ls_epoch: stale, m_default_app: 'totp' });
      applyLsEpoch(storage);
      expect(storage.snapshot()).toEqual({ veydan_ls_epoch: '5' });
    }
  });

  it('marks an empty storage', () => {
    const storage = new MemoryStorage();
    applyLsEpoch(storage);
    expect(storage.snapshot()).toEqual({ veydan_ls_epoch: '5' });
  });

  it('keeps the keys of the current epoch', () => {
    const kept = { veydan_ls_epoch: '5', vb_locale: 'ru', m_default_app: 'notes' };
    const storage = new MemoryStorage(kept);
    applyLsEpoch(storage);
    applyLsEpoch(storage);
    expect(storage.snapshot()).toEqual(kept);
    expect(storage.clears).toBe(0);
  });
});
