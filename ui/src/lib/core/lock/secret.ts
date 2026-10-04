// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import type { LockKind } from '$lib/core/types';

export const LOCK_MIN_LEN = 4;

export type LockSecretIssue = 'short' | 'digits' | 'mismatch';

/** Mirrors the backend rules: min length, digits-only PIN, matching confirmation. */
export function lockSecretIssue(kind: LockKind, password: string, confirm: string): LockSecretIssue | null {
  const value = password.trim();
  if (value.length < LOCK_MIN_LEN) return 'short';
  if (kind === 'pin' && !/^\d+$/.test(value)) return 'digits';
  if (value !== confirm.trim()) return 'mismatch';
  return null;
}

export interface LockInputAttrs {
  inputmode: 'numeric' | 'text';
  pattern: string | undefined;
}

/** Input attributes that open the right keyboard for the secret kind. */
export function lockInputAttrs(kind: LockKind): LockInputAttrs {
  return kind === 'pin'
    ? { inputmode: 'numeric', pattern: '[0-9]*' }
    : { inputmode: 'text', pattern: undefined };
}
