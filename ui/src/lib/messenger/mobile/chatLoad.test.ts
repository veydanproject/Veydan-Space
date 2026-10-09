// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { shouldLoad, type ChatPageState } from './chatLoad';

const A = `dm:${'a'.repeat(64)}`;
const B = `group:${'b'.repeat(64)}`;

/** The page showing chat A, loaded at epoch 0. */
const shown: ChatPageState = { id: A, loading: false, loadedFor: A, loadedEpoch: 0, resets: 0, active: true, missingId: null, known: true };

describe('shouldLoad', () => {
  it('loads a chat new to the page', () => {
    expect(shouldLoad({ ...shown, loadedFor: null, active: false })).toBe(true);
    expect(shouldLoad({ ...shown, id: B })).toBe(true);
  });

  it('leaves the chat it shows alone', () => {
    expect(shouldLoad(shown)).toBe(false);
  });

  it('waits for a load under way and for an address with a chat', () => {
    expect(shouldLoad({ ...shown, loadedFor: null, loading: true })).toBe(false);
    expect(shouldLoad({ ...shown, id: '', loadedFor: null })).toBe(false);
  });

  it('does not reopen a chat closed on purpose (deleted, forgotten, archived)', () => {
    expect(shouldLoad({ ...shown, active: false })).toBe(false);
    // The list read after a delete no longer has it.
    expect(shouldLoad({ ...shown, active: false, known: false })).toBe(false);
  });

  it('loads it again once the store was emptied under the page', () => {
    expect(shouldLoad({ ...shown, active: false, known: false, resets: 1 })).toBe(true);
  });

  it('does not spin while the chat is missing, and loads it when it turns up', () => {
    const missing = { ...shown, active: false, known: false, missingId: A, loadedEpoch: 1, resets: 1 };
    expect(shouldLoad(missing)).toBe(false);
    // Still locked: a second reset does not loop while it is missing.
    expect(shouldLoad({ ...missing, resets: 2 })).toBe(false);
    expect(shouldLoad({ ...missing, known: true })).toBe(true);
  });
});
