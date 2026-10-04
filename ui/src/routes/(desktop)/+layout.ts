// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Desktop-only routes (workspaces, proxies, terminal, files, quick capture).
import { redirect } from '@sveltejs/kit';
import { isMobile } from '$lib/core/platform';

export const ssr = false;

export function load() {
  if (isMobile) redirect(307, '/');
}
