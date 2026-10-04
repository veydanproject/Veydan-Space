// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The generator's history, shared by the desktop generator and the phone's
// screen: what it asks the backend and what it keeps in memory.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { HISTORY_LIMITS, historyLimitFromId, historyLimitId } from '$lib/pass/password-gen';

type Row = { id: string; password: string; created_at: string };

const db = vi.hoisted(() => ({ rows: [] as Row[], fail: false }));
const pwgen = vi.hoisted(() => ({
  list: vi.fn(async () => {
    if (db.fail) throw new Error('db');
    return [...db.rows].sort((a, b) => b.created_at.localeCompare(a.created_at));
  }),
  add: vi.fn(async (password: string) => {
    const row = { id: `h-${db.rows.length + 1}`, password, created_at: `2026-10-04T10:00:0${db.rows.length}Z` };
    db.rows.push(row);
    return row;
  }),
  trim: vi.fn(async (limit: number) => {
    const keep = new Set([...db.rows].sort((a, b) => b.created_at.localeCompare(a.created_at)).slice(0, limit).map((r) => r.id));
    db.rows = db.rows.filter((r) => keep.has(r.id));
  }),
  clear: vi.fn(async () => {
    if (db.fail) throw new Error('db');
    db.rows = [];
  }),
}));
vi.mock('$lib/pass/api', () => ({ api: { pwgen } }));

const { pwgenHistory } = await import('./pwgen-history.svelte');
const { confirmState } = await import('$lib/core/ui/confirm.svelte');

describe('the generator history store', () => {
  beforeEach(() => {
    db.rows = [];
    db.fail = false;
    pwgenHistory.forget();
    vi.clearAllMocks();
  });

  it('shows nothing before it has read the database', async () => {
    db.rows = [{ id: 'a', password: 'x', created_at: '2026-10-04T09:00:00Z' }];
    expect(pwgenHistory.loaded).toBe(false);
    expect(pwgenHistory.entries).toEqual([]);
    await pwgenHistory.load();
    expect(pwgenHistory.loaded).toBe(true);
    expect(pwgenHistory.entries.map((e) => e.id)).toEqual(['a']);
  });

  it('records a password newest first and lists again', async () => {
    await pwgenHistory.record('first', null);
    await pwgenHistory.record('second', null);
    expect(pwgenHistory.entries.map((e) => e.password)).toEqual(['second', 'first']);
    expect(pwgen.trim).not.toHaveBeenCalled();
  });

  it('keeps the newest `limit` passwords when there is a limit', async () => {
    for (const pw of ['a', 'b', 'c']) await pwgenHistory.record(pw, 2);
    expect(pwgen.trim).toHaveBeenLastCalledWith(2);
    expect(pwgenHistory.entries.map((e) => e.password)).toEqual(['c', 'b']);
  });

  it('clears the database and the list', async () => {
    await pwgenHistory.record('a', null);
    await pwgenHistory.clear();
    expect(pwgen.clear).toHaveBeenCalledOnce();
    expect(pwgenHistory.entries).toEqual([]);
  });

  it('keeps the list when the backend refuses to clear it', async () => {
    await pwgenHistory.record('a', null);
    db.fail = true;
    await pwgenHistory.clear();
    expect(pwgenHistory.entries.map((e) => e.password)).toEqual(['a']);
  });

  it('forgets the passwords in memory without touching the database', async () => {
    await pwgenHistory.record('a', null);
    pwgenHistory.forget();
    expect(pwgenHistory.entries).toEqual([]);
    expect(pwgenHistory.loaded).toBe(false);
    expect(db.rows).toHaveLength(1);
    expect(pwgen.clear).not.toHaveBeenCalled();
  });

  it('forgets when the last generator lets go, not before', async () => {
    const drawer = pwgenHistory.hold();
    const pane = pwgenHistory.hold();
    await pwgenHistory.record('a', null);
    pane();
    pane(); // a second release of the same hold counts once
    expect(pwgenHistory.entries.map((e) => e.password)).toEqual(['a']);
    drawer();
    expect(pwgenHistory.entries).toEqual([]);
    expect(pwgenHistory.loaded).toBe(false);
    expect(db.rows).toHaveLength(1);
  });

  it('drops a list that answers after the last generator let go', async () => {
    db.rows = [{ id: 'a', password: 'secret', created_at: '2026-10-04T09:00:00Z' }];
    let answer!: () => void;
    pwgen.list.mockImplementationOnce(
      () => new Promise((resolve) => (answer = () => resolve([...db.rows]))),
    );
    const release = pwgenHistory.hold();
    const pending = pwgenHistory.load();
    release(); // the lock unmounts the generator while the list is read
    answer();
    await pending;
    expect(pwgenHistory.entries).toEqual([]);
    expect(pwgenHistory.loaded).toBe(false);

    // A later hold reads it again.
    const again = pwgenHistory.hold();
    await pwgenHistory.load();
    expect(pwgenHistory.entries.map((e) => e.id)).toEqual(['a']);
    again();
  });

  it('does not read the list back for a password saved across the lock', async () => {
    let added!: () => void;
    pwgen.add.mockImplementationOnce(
      (password: string) =>
        new Promise((resolve) => {
          added = () => {
            const row = { id: 'h-1', password, created_at: '2026-10-04T10:00:00Z' };
            db.rows.push(row);
            resolve(row);
          };
        }),
    );
    const release = pwgenHistory.hold();
    const pending = pwgenHistory.record('secret', null);
    release();
    added();
    await pending;
    expect(db.rows.map((r) => r.password)).toEqual(['secret']);
    expect(pwgen.list).not.toHaveBeenCalled();
    expect(pwgenHistory.entries).toEqual([]);
    expect(pwgenHistory.loaded).toBe(false);
  });

  it('stays unloaded when the backend cannot list', async () => {
    db.fail = true;
    await pwgenHistory.load();
    expect(pwgenHistory.loaded).toBe(false);
    expect(pwgenHistory.entries).toEqual([]);
  });
});

describe('the question before the history is cleared', () => {
  const tr = (key: string) => `«${key}»`;

  beforeEach(async () => {
    db.rows = [{ id: 'a', password: 'p!=q', created_at: '2026-10-04T09:00:00Z' }];
    db.fail = false;
    pwgenHistory.forget();
    await pwgenHistory.load();
    vi.clearAllMocks();
  });

  it('asks with the same words on a computer and a phone, then clears on yes', async () => {
    const answer = pwgenHistory.confirmClear(tr);
    expect(confirmState.current).toMatchObject({
      title: '«pwgen_history_clear_title»',
      message: '«pwgen_history_clear_message»',
      confirmLabel: '«pwgen_btn_clear»',
    });
    // A deletion: the danger button (the default of `ask()`).
    expect(confirmState.current?.variant).toBeUndefined();
    expect(pwgen.clear).not.toHaveBeenCalled();
    confirmState.answer(true);
    expect(await answer).toBe(true);
    expect(pwgen.clear).toHaveBeenCalledTimes(1);
    expect(db.rows).toEqual([]);
    expect(pwgenHistory.entries).toEqual([]);
  });

  it('deletes nothing when the answer is no (Cancel, Escape, the backdrop)', async () => {
    const answer = pwgenHistory.confirmClear(tr);
    confirmState.answer(false);
    expect(await answer).toBe(false);
    expect(pwgen.clear).not.toHaveBeenCalled();
    expect(db.rows.map((r) => r.id)).toEqual(['a']);
    expect(pwgenHistory.entries.map((e) => e.id)).toEqual(['a']);
  });

  it('a second question answers the first one no', async () => {
    const first = pwgenHistory.confirmClear(tr);
    const second = pwgenHistory.confirmClear(tr);
    expect(await first).toBe(false);
    confirmState.answer(true);
    expect(await second).toBe(true);
    expect(pwgen.clear).toHaveBeenCalledTimes(1);
  });
});

describe('the history limit', () => {
  it('maps every choice to its limit and back', () => {
    for (const o of HISTORY_LIMITS) expect(historyLimitId(historyLimitFromId(o.id))).toBe(o.id);
    expect(historyLimitFromId('unlimited')).toBeNull();
    expect(historyLimitFromId('25')).toBe(25);
    expect(historyLimitId(null)).toBe('unlimited');
    expect(historyLimitId(100)).toBe('100');
  });

  it('reads anything that is not a choice as no limit', () => {
    for (const id of [null, undefined, '', 'abc', '0', '-5']) expect(historyLimitFromId(id), String(id)).toBeNull();
  });

  it('offers no limit first, then 10, 25, 50 and 100', () => {
    expect(HISTORY_LIMITS.map((o) => o.id)).toEqual(['unlimited', '10', '25', '50', '100']);
  });
});
