// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The shapes of the archive of chats as the UI reads them: the payload of the
// event `archive.progress` and the answer of `messenger_archive_status`
// (crates/messenger/archive/src/lib.rs, `ArchiveProgress`, `PauseReason`).
// Written by hand until `make msg-types` writes them into `generated/`.

export type ArchivePhase = 'idle' | 'restoring' | 'writing' | 'paused';

export type PauseReason =
  /** Grace or suspended: reading goes on; `until` is unix seconds when the server says. */
  | { why: 'read_only'; until?: number }
  | { why: 'quota' }
  | { why: 'offline' }
  | { why: 'no_storage' };

export interface ArchiveProgress {
  phase: ArchivePhase;
  /** Segments done and in all, in the current phase. */
  done: number;
  total: number;
  bytes_done: number;
  bytes_total: number;
  /** The month under way, `yyyy-mm`. */
  month?: string;
  /** Why the archive stands, in `paused`. */
  reason?: PauseReason;
}

export interface ArchiveStatus {
  /** False where the device has no storage the archive can use (the step is skipped, the card says so). */
  available: boolean;
  progress: ArchiveProgress;
  size_bytes: number;
  /** `yyyy-mm` of the oldest and the newest month in the archive. */
  from_month: string | null;
  to_month: string | null;
  paused: PauseReason | null;
  /** The scopes (`dm`, `dm@<server>`, `group:<id>`) with months still to come: an open chat of one says so. */
  restoring_scopes: string[];
  /** The months (`yyyy-mm`) this device does not restore, their files over the bounds of the format: the card names them. */
  too_large: string[];
  /** The months (`yyyy-mm`) this device holds and does not write, over the bounds of the format: in no backup, the card names them. */
  not_backed_up: string[];
}

export const IDLE_PROGRESS: ArchiveProgress = { phase: 'idle', done: 0, total: 0, bytes_done: 0, bytes_total: 0 };
