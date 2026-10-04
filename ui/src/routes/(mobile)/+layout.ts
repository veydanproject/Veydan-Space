// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Mobile-only routes (note editor stack, search, tags, tools, TOTP, sync settings).
import { redirect } from '@sveltejs/kit';
import { isDesktop } from '$lib/core/platform';

export const ssr = false;

export function load() {
  if (isDesktop) redirect(307, '/');
}
