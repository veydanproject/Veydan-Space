// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What a link outside shows before anyone follows it: where it really
// leads, and what about it deserves a second look. Nothing is asked of
// the network.

import type { ExternalUrl } from './types';

export type LinkRisk =
  /** `http`: anyone on the way reads and changes the page. */
  | 'insecure'
  /** Letters of another alphabet that may look like a known name. */
  | 'lookalike'
  /** A number instead of a name. */
  | 'address'
  /** `https://known.example@other.example`: leads to the second one. */
  | 'userinfo'
  /** Not the usual port. */
  | 'port';

export interface ExternalInfo {
  /** Host as the browser will ask for it (letters of other alphabets in their encoded form). */
  host: string;
  /** Path and query, shortened; empty for the front page. */
  path: string;
  secure: boolean;
  risks: LinkRisk[];
}

const MAX_PATH_CHARS = 60;

export function describeExternal(url: ExternalUrl): ExternalInfo {
  const u = new URL(url);
  const host = u.hostname;
  const secure = u.protocol === 'https:';
  const risks: LinkRisk[] = [];
  if (!secure) risks.push('insecure');
  if (host.split('.').some((label) => label.startsWith('xn--'))) risks.push('lookalike');
  if (/^\d+(\.\d+){3}$/.test(host) || host.startsWith('[')) risks.push('address');
  if (u.username || u.password) risks.push('userinfo');
  if (u.port) risks.push('port');

  let path = u.pathname === '/' ? '' : u.pathname;
  path += u.search;
  try { path = decodeURI(path); } catch { /* shown as written */ }
  const chars = [...path];
  if (chars.length > MAX_PATH_CHARS) path = `${chars.slice(0, MAX_PATH_CHARS - 1).join('')}…`;

  return { host: u.port ? `${host}:${u.port}` : host, path, secure, risks };
}

/** Only `https` pages are asked for a preview. */
export function canPreview(url: ExternalUrl): boolean {
  const info = describeExternal(url);
  return info.secure && !info.risks.includes('address') && !info.risks.includes('userinfo');
}
