// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The module Communities as the mobile shell sees it: a tab of the
// messenger's bar («Каналы») and its full-screen pages, both the
// messenger's, given through its seam.

import type { ModuleDef } from '$lib/core/module';
import { common } from './common';

export const module: ModuleDef = {
  ...common,
  nav: [],
  settings: [],
};
