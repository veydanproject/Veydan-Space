// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import type { ClientInit } from '@sveltejs/kit';
import { applyLsEpoch } from '$lib/core/ls-epoch';

// SvelteKit awaits `init` before it fetches any route module. Stores read
// localStorage while their modules are evaluated, and the modules of a page
// load in parallel with the root layout, so this is the only point that is
// ahead of every read.
export const init: ClientInit = () => {
  applyLsEpoch(localStorage);
};
