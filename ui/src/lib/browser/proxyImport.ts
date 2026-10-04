// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import type { BulkProxyItem } from '$lib/browser/types';

export type ProxyImportType = 'http' | 'https' | 'socks5';

export interface ParsedProxyOk {
  ok: true;
  lineNumber: number;
  raw: string;
  row: Omit<BulkProxyItem, 'line_number'>;
}

export interface ParsedProxyError {
  ok: false;
  lineNumber: number;
  raw: string;
  errorKey: 'proxy_import_err_format' | 'proxy_import_err_scheme' | 'proxy_import_err_port' | 'proxy_import_err_host';
}

export type ParsedProxyLine = ParsedProxyOk | ParsedProxyError;

const SCHEME_RE = /^([a-z0-9+.-]+):\/\//i;
const SCHEME_MAP: Record<string, ProxyImportType> = {
  http: 'http',
  https: 'https',
  socks5: 'socks5',
  socks5h: 'socks5',
};

function parsePort(s: string): number | null {
  if (!/^\d{1,5}$/.test(s)) return null;
  const port = Number(s);
  return port >= 1 && port <= 65535 ? port : null;
}

function validHost(s: string): boolean {
  return s.length > 0 && !/\s/.test(s);
}

/** Parse `host:port` where host may be a bracketed IPv6 literal (`[::1]:8080`). */
function parseHostPort(s: string): { host: string; port: number } | null {
  let host: string;
  let portStr: string;
  if (s.startsWith('[')) {
    const m = /^\[([^\]]+)\]:(\d{1,5})$/.exec(s);
    if (!m) return null;
    host = `[${m[1]}]`;
    portStr = m[2];
  } else {
    const parts = s.split(':');
    if (parts.length !== 2) return null;
    [host, portStr] = parts;
  }
  const port = parsePort(portStr);
  if (port === null || !validHost(host)) return null;
  return { host, port };
}

function parseLine(
  raw: string,
  lineNumber: number,
  defaultType: ProxyImportType,
): ParsedProxyLine | null {
  const line = raw.trim();
  if (!line || line.startsWith('#')) return null;

  const err = (errorKey: ParsedProxyError['errorKey']): ParsedProxyError => ({
    ok: false,
    lineNumber,
    raw: line,
    errorKey,
  });

  let proxyType = defaultType;
  let rest = line;
  const schemeMatch = SCHEME_RE.exec(line);
  if (schemeMatch) {
    const mapped = SCHEME_MAP[schemeMatch[1].toLowerCase()];
    if (!mapped) return err('proxy_import_err_scheme');
    proxyType = mapped;
    rest = line.slice(schemeMatch[0].length);
  }
  if (!rest) return err('proxy_import_err_format');

  let host: string;
  let port: number;
  let username: string | null = null;
  let password: string | null = null;

  const at = rest.lastIndexOf('@');
  if (at !== -1) {
    // `user:pass@host:port` or `host:port@user:pass` — the side that parses
    // as host:port (numeric port) wins; passwords with '@' survive lastIndexOf.
    const left = rest.slice(0, at);
    const right = rest.slice(at + 1);
    const rightHp = parseHostPort(right);
    const leftHp = rightHp ? null : parseHostPort(left);
    const hp = rightHp ?? leftHp;
    if (!hp) return err('proxy_import_err_format');
    ({ host, port } = hp);
    const cred = rightHp ? left : right;
    const sep = cred.indexOf(':');
    if (sep === -1) {
      username = cred;
    } else {
      username = cred.slice(0, sep);
      password = cred.slice(sep + 1);
    }
    if (!username) return err('proxy_import_err_format');
  } else if (rest.startsWith('[')) {
    const hp = parseHostPort(rest);
    if (!hp) return err('proxy_import_err_format');
    ({ host, port } = hp);
  } else {
    const parts = rest.split(':');
    if (parts.length === 2) {
      const hp = parseHostPort(rest);
      if (!hp) {
        return parsePort(parts[1]) === null ? err('proxy_import_err_port') : err('proxy_import_err_host');
      }
      ({ host, port } = hp);
    } else if (parts.length === 4) {
      // host:port:user:pass
      const p = parsePort(parts[1]);
      if (p === null) return err('proxy_import_err_port');
      if (!validHost(parts[0])) return err('proxy_import_err_host');
      host = parts[0];
      port = p;
      username = parts[2];
      password = parts[3];
      if (!username) return err('proxy_import_err_format');
    } else {
      return err('proxy_import_err_format');
    }
  }

  return {
    ok: true,
    lineNumber,
    raw: line,
    row: { proxy_type: proxyType, host, port, username, password },
  };
}

/**
 * Parse a proxy list (one proxy per line) into rows for `proxies_bulk_create`.
 * Supported: `host:port`, `host:port:user:pass`, `user:pass@host:port`,
 * `host:port@user:pass`, each optionally prefixed with `scheme://`
 * (http | https | socks5 | socks5h). Blank lines and `#` comments are skipped.
 */
export function parseProxyList(text: string, defaultType: ProxyImportType): ParsedProxyLine[] {
  const out: ParsedProxyLine[] = [];
  const lines = text.split('\n');
  for (let i = 0; i < lines.length; i++) {
    const parsed = parseLine(lines[i], i + 1, defaultType);
    if (parsed) out.push(parsed);
  }
  return out;
}

export function toBulkItems(lines: ParsedProxyLine[]): BulkProxyItem[] {
  return lines
    .filter((l): l is ParsedProxyOk => l.ok)
    .map((l) => ({ line_number: l.lineNumber, ...l.row }));
}
