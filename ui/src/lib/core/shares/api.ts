// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What another app shared into Veydan through the system's share sheet
// (crates/shell/src/shares.rs): a text, files, or both. The shell keeps one
// share at a time, in memory; its files are copies in the app's cache that
// the module that takes the share reads by their paths (as files the user
// picked) and lets go of with `shares_forget` once it is done with them (a
// module calls that command through its own api: the messenger reaches the
// core only by `$lib/core/module`, where the types live).

import { call } from '../api';
import type { IncomingShare } from '../module';

export type { IncomingShare, SharedFile } from '../module';

export const sharesApi = {
  /** The share that waits, taken: nothing when none came (a computer: never). */
  take: () => call<IncomingShare | null>('shares_take'),
  /** Sent, or given up: the copies go. */
  forget: (id: string) => call<void>('shares_forget', { id }),
};
