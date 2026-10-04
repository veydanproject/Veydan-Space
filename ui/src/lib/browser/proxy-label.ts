// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import type { Proxy } from '$lib/browser/types';

/**
 * Label for a proxy in select dropdowns: the name plus the geo assigned by the
 * proxy check, e.g. "175.110.115.169:10525 (RU, Moscow)". Names usually already
 * contain host:port, so the URL is not repeated; without geo it's just the name.
 */
export function proxyOptionLabel(p: Proxy): string {
  const geo = [p.country, p.city].filter(Boolean).join(', ');
  return geo ? `${p.name} (${geo})` : p.name;
}
