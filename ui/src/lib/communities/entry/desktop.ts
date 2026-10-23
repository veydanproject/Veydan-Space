// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The module Communities as the desktop shell sees it: a section of the
// messenger's page, nothing of its own in the shell.

import type { ModuleDef } from '$lib/core/module';
import { common } from './common';

export const module: ModuleDef = {
  ...common,
  nav: [],
  // The module's switch is in the Modules section of the core (Space only).
  settings: [],
};
