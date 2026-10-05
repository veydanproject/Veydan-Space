// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import type { Proxy } from '$lib/browser/types';
import type { TranslationKey } from '$lib/core/i18n';

type Tr = (key: TranslationKey) => string;

/** A Tor row names no server: the app runs its own tor and routes the traffic through it. */
export const isTor = (p: { proxy_type: string }): boolean => p.proxy_type === 'tor';

/**
 * The exit countries of a Tor row as the backend keeps them (`de,nl`), for
 * display: `DE, NL`; empty when any country is allowed.
 */
export function torCountries(country: string | null | undefined): string {
  return (country ?? '')
    .split(',')
    .map((c) => c.trim().toUpperCase())
    .filter(Boolean)
    .join(', ');
}

/** `Tor · DE, NL` or `Tor · any country`: what a Tor row shows where others show host:port. */
export function torLabel(country: string | null | undefined, tr: Tr): string {
  return `Tor · ${torCountries(country) || tr('proxy_tor_any')}`;
}

/** What a proxy row shows as its address: host:port, or the Tor label for a Tor row. */
export function proxyAddress(p: Proxy, tr: Tr): string {
  return isTor(p) ? torLabel(p.country, tr) : `${p.host}:${p.port}`;
}

/**
 * Label for a proxy in select dropdowns: the name plus the geo assigned by the
 * proxy check, e.g. "175.110.115.169:10525 (RU, Moscow)". Names usually already
 * contain host:port, so the URL is not repeated; without geo it's just the name.
 * A Tor row has no geo of its own: its country is the set of exit countries.
 */
export function proxyOptionLabel(p: Proxy, tr: Tr): string {
  if (isTor(p)) return `${p.name} (${torLabel(p.country, tr)})`;
  const geo = [p.country, p.city].filter(Boolean).join(', ');
  return geo ? `${p.name} (${geo})` : p.name;
}

/** The short country chip of a proxy: its country, `DE, NL` for the exit countries of Tor, `Tor` for any country; null when none. */
export function proxyChip(p: Proxy): string | null {
  if (isTor(p)) return torCountries(p.country) || 'Tor';
  return p.country || null;
}

/** The type as the lists print it: `Tor`, the others as the backend names them. */
export const proxyTypeName = (p: { proxy_type: string }): string => (isTor(p) ? 'Tor' : p.proxy_type);
