// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The levels and sections of a channel (communities-wire §14, spec §5) as
// the commands of the module give them. Written by hand until the Rust
// side generates them (generated/channels.ts) under the same names; the
// fields stage 2 adds to `ChannelDetail`, a staff entry and a feed item are
// read as optional here, so a page works with a core of before them.

import type { ChannelDetail, FeedItemView, StaffView } from '../api';
import type { LevelView, ScopeView } from '../generated/channels';

/** `sd-v1` for sections of readers; `flat-v1` for small service ones (≤ 300 people). */
export type ScopeScheme = 'sd-v1' | 'flat-v1';
/** How much of a section's past a newcomer to it reads. */
export type ScopeHistory = 'open' | 'from_join';

/** The views of the levels and the sections, as the Rust side writes them (`generated/channels.ts`). */
export type { LevelInput, LevelRequestView, LevelRights, LevelView, NewScopeInput, ScopeSetInput, ScopeView } from '../generated/channels';

/** What `ChannelDetail` gains. */
export interface DetailLevels {
  scopes: ScopeView[];
  levels: LevelView[];
  my_levels: number[];
}

export type ChannelDetailLv = ChannelDetail & Partial<DetailLevels>;
export type StaffViewLv = StaffView & { levels?: number[] };
export type FeedItemViewLv = FeedItemView & { scope_id?: string };
