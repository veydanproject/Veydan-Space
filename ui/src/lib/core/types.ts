// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Types of the shell's own commands and errors; a module's types live in its `types.ts` (platform-spec 11.5).

export type AppError = {
  code: 'db' | 'io' | 'browser' | 'proxy' | 'not_found' | 'conflict_changed' | 'vault_locked' | 'vault_mismatch' | 'decrypt_failed' | 'recovery_invalid' | 'other';
  message: string;
};

export interface LockStatus {
  enabled: boolean;
  locked: boolean;
  /** Inactivity minutes before auto-lock; 0 disables auto-lock */
  timeout_min: number;
  /** `none` until the first password, `ok` when the vault exists, `mismatch` when the lock cannot unwrap it */
  vault: 'none' | 'ok' | 'mismatch';
  kind: LockKind;
  hint: string | null;
  /** A recovery key wraps the vault; a lost PIN/password can be replaced with it. */
  has_recovery: boolean;
}

export type LockKind = 'pin' | 'password';

export interface LockSecret {
  password: string;
  kind: LockKind;
  hint: string | null;
}

/** Lock status plus the recovery key when one was just created; it is shown once. */
export interface LockSetResult extends LockStatus {
  recovery_key: string | null;
}

export interface HostInfo {
  os: string;
  arch: string;
  version: string;
}

/** The answer of `update_check` (crates/shell/src/update.rs): the product's
 *  latest.json against this build. */
export interface UpdateCheck {
  current: string;
  /** The version latest.json offers, as of `checked_at`; null before a first check. */
  latest: string | null;
  available: boolean;
  /** Seconds since the Unix epoch. */
  checked_at: number | null;
  /** The release page of `latest`. */
  page: string | null;
  /** Set when the call did not ask the network: the last check is recent, or the network is metered. */
  skipped: 'recent' | 'metered' | null;
}
