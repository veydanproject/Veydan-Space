// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import type { MessengerRuntimeStatus } from '../api';

/** What the dot beside the title shows. `on` from the start and after a
 *  wake; `connecting` after 10 s without relays, `lost` after a minute
 *  (the runtime counts, `link`). Silent and locked say why it is quiet. */
export type NetState = 'off' | 'silent' | 'locked' | 'on' | 'connecting' | 'lost';

export function netState(rt: MessengerRuntimeStatus | null | undefined): NetState {
  if (!rt) return 'off';
  if (rt.silent_mode) return 'silent';
  if (!rt.session_active) return 'locked';
  switch (rt.link) {
    case 'waiting': return 'connecting';
    case 'lost': return 'lost';
    default: return 'on';
  }
}

/** The modifiers of the common `.status-dot` (base.css) for a state: green on, amber pulsing
 *  while it connects, red when lost; grey (none) when off, silent or locked. */
export function netDot(state: NetState): string {
  switch (state) {
    case 'on': return 'ok';
    case 'connecting': return 'warn pulse';
    case 'lost': return 'bad';
    default: return '';
  }
}
