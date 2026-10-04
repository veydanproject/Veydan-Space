// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Bottom navigation: the root shell has one bar; a module may claim its own
// for its screens (`ModuleDef.bar`, through the registry).

import type { NavItem } from '$lib/core/module';
import { loadDefaultApp, registry } from '$lib/core/registry';

export type { NavItem };

/**
 * The root bar. Space: Home, which opens the default module when one is
 * set, and Settings. A product of one module has no Home (platform-spec
 * 11.7): its bar is that module's own screens, then Settings.
 */
export function rootNav(): NavItem[] {
  const settings: NavItem = { id: 'settings', title: 'app_settings', icon: 'settings', href: '/settings' };
  const only = registry.only;
  if (only && only.nav.length > 0) return [...only.nav, settings];
  const app = registry.navItem(loadDefaultApp());
  return [{ id: 'home', title: 'nav_home', icon: 'home', href: app?.href ?? (only && registry.homeRoute(only)) ?? '/' }, settings];
}

/**
 * Whether the product has a Home screen behind `/` for a root screen's back
 * arrow to lead to. A product of one module has none: `/` is the module's
 * first screen, and the root screens are tabs of the bar, without an arrow.
 */
export function hasHome(): boolean {
  return registry.only === undefined;
}

/** The bar for a route: a module's own, the root one, or `null` to hide it (note editor, scanner, chat). */
export function navItems(url: URL): NavItem[] | null {
  const own = registry.bar(url);
  return own === undefined ? rootNav() : own;
}

/** Active item of a bar: the one whose route is the longest prefix of the path (`/` only exactly). */
export function navActive(pathname: string, items: NavItem[]): string {
  let best: NavItem | undefined;
  for (const it of items) {
    const href = it.href?.split('?')[0];
    if (!href) continue;
    const hit = href === '/' ? pathname === '/' : pathname === href || pathname.startsWith(href + '/');
    if (hit && (!best || href.length > (best.href?.split('?')[0].length ?? 0))) best = it;
  }
  return best?.id ?? '';
}
