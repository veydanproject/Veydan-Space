// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The drafts kept on the device (ROO-1112): a change is written within
// SAVE_MS and at once when the page is hidden; a sent or emptied field
// forgets the draft at once; a composer that opens empty before the kept
// drafts came in forgets nothing (an open one takes it); one chat's draft
// is not another's; a late load does not bring back a sent or emptied
// draft, nor one of a session reset since.

import { flushSync, untrack } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({
  get: vi.fn(async () => [] as { chat: string; text: string; md: boolean }[]),
  set: vi.fn(async () => {}),
}));
vi.mock('../api', () => ({ messengerApi: { drafts: api } }));
// The page's events, without a DOM.
const doc = Object.assign(new EventTarget(), { visibilityState: 'visible' });
const win = new EventTarget();
vi.stubGlobal('document', doc);
vi.stubGlobal('window', win);

const { drafts, draftsLoaded, flushDrafts, loadDrafts, markdownDrafts, resetDrafts, setDraft } = await import('./drafts.svelte');

function effectsRun() {
  let runs = 0;
  const stop = $effect.root(() => { $effect(() => { runs++; }); });
  flushSync();
  stop();
  return runs > 0;
}

beforeEach(() => {
  vi.useFakeTimers();
  resetDrafts();
  api.get.mockClear();
  api.set.mockClear();
});
afterEach(() => vi.useRealTimers());

describe('drafts', () => {
  it('writes the last text of a run of changes once, after a short wait', () => {
    setDraft('a', 'Q', false);
    setDraft('a', 'QA', false);
    setDraft('a', 'QA1105 draft', false);
    expect(api.set).not.toHaveBeenCalled();
    vi.advanceTimersByTime(400);
    expect(api.set).toHaveBeenCalledTimes(1);
    expect(api.set).toHaveBeenCalledWith('a', 'QA1105 draft', false);
  });

  it('writes at once when the page is hidden or left, and only what is owed', () => {
    setDraft('a', 'QA1105 draft', true);
    doc.visibilityState = 'hidden';
    doc.dispatchEvent(new Event('visibilitychange'));
    doc.visibilityState = 'visible';
    expect(api.set).toHaveBeenCalledWith('a', 'QA1105 draft', true);
    setDraft('a', 'QA1105 draft!', true);
    win.dispatchEvent(new Event('pagehide'));
    expect(api.set).toHaveBeenLastCalledWith('a', 'QA1105 draft!', true);
    win.dispatchEvent(new Event('blur'));
    expect(api.set).toHaveBeenCalledTimes(2);
  });

  it('forgets a sent or emptied field at once, and the owed write does not bring it back', () => {
    setDraft('a', 'QA1105 draft', false);
    setDraft('a', '', false);
    expect(api.set).toHaveBeenCalledWith('a', '', false);
    vi.advanceTimersByTime(400);
    expect(api.set).toHaveBeenCalledTimes(1);
    expect(drafts.has('a')).toBe(false);
  });

  it('an empty field that opens before the drafts came in forgets nothing', async () => {
    setDraft('a', '', false);
    expect(api.set).not.toHaveBeenCalled();
    api.get.mockResolvedValueOnce([{ chat: 'a', text: 'QA1105 draft', md: true }]);
    const n = draftsLoaded.n;
    await loadDrafts();
    expect(draftsLoaded.n).toBe(n + 1);
    expect(drafts.get('a')).toBe('QA1105 draft');
    expect(markdownDrafts.has('a')).toBe(true);
    expect(drafts.has('b')).toBe(false);
  });

  it('a draft typed before the kept ones came in stands', async () => {
    setDraft('a', 'new', false);
    api.get.mockResolvedValueOnce([{ chat: 'a', text: 'old', md: false }, { chat: 'b', text: 'other', md: false }]);
    await loadDrafts();
    expect(drafts.get('a')).toBe('new');
    expect(drafts.get('b')).toBe('other');
    flushDrafts();
    expect(api.set).toHaveBeenCalledWith('a', 'new', false);
  });

  // The two effects of Composer: the field writes its text, the late drafts
  // fill an empty field. Effects run only on Svelte's client build; vitest
  // takes the server one here, where this would pass without trying.
  it.runIf(effectsRun())('an open empty composer takes the kept draft when it comes in, and writes nothing', async () => {
    let text = $state('');
    const stop = $effect.root(() => {
      $effect(() => { setDraft('a', text, false); });
      $effect(() => {
        void draftsLoaded.n;
        untrack(() => { if (!text) text = drafts.get('a') ?? ''; });
      });
    });
    try {
      flushSync();
      api.get.mockResolvedValueOnce([{ chat: 'a', text: 'QA1105 draft', md: false }]);
      await loadDrafts();
      flushSync();
      expect(text).toBe('QA1105 draft');
      expect(drafts.get('a')).toBe('QA1105 draft');
      vi.advanceTimersByTime(400);
      expect(api.set).not.toHaveBeenCalled();
    } finally {
      stop();
    }
  });

  it('a draft sent or emptied while the kept ones were on their way does not come back', async () => {
    let answer!: (v: { chat: string; text: string; md: boolean }[]) => void;
    api.get.mockReturnValueOnce(new Promise((r) => { answer = r; }));
    const loading = loadDrafts();
    setDraft('a', 'new', false);
    setDraft('a', '', false);
    answer([{ chat: 'a', text: 'old', md: false }, { chat: 'b', text: 'other', md: false }]);
    await loading;
    expect(drafts.has('a')).toBe(false);
    expect(drafts.get('b')).toBe('other');
  });

  it('a load answered after a reset brings nothing into the next session', async () => {
    let answer!: (v: { chat: string; text: string; md: boolean }[]) => void;
    api.get.mockReturnValueOnce(new Promise((r) => { answer = r; }));
    const n = draftsLoaded.n;
    const loading = loadDrafts();
    resetDrafts();
    answer([{ chat: 'a', text: 'old session', md: true }]);
    await loading;
    expect(drafts.size).toBe(0);
    expect(markdownDrafts.size).toBe(0);
    expect(draftsLoaded.n).toBe(n);
  });
});
