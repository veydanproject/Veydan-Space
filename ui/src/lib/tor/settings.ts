// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The settings form of the Tor page, kept apart from the markup so it can be
// tested: the lists the user types as text, their checks and the way from the
// form to `TorSettings` and back. The backend checks again; these checks only
// spare a round trip and say which field is wrong.

import type { BridgeMode, TorSettings } from '$lib/tor/types';

/** The pieces of a comma list: split on commas, semicolons and blanks, empty pieces dropped. */
export function splitList(text: string): string[] {
  return text.split(/[\s,;]+/).filter(Boolean);
}

/** One entry of a list per row, blank rows dropped. */
export function splitLines(text: string): string[] {
  return text.split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
}

export function joinList(items: readonly (string | number)[]): string {
  return items.join(', ');
}

export function joinLines(lines: readonly string[]): string {
  return lines.join('\n');
}

/** Two ASCII letters, as ISO 3166 alpha-2 codes are written. */
export function isCountryCode(code: string): boolean {
  return /^[a-zA-Z]{2}$/.test(code);
}

export type Parsed<T> = { value: T; invalid: string[] };

/** "DE, nl de" → `de`, `nl`: lower case, no repeats, order kept; wrong pieces are reported. */
export function parseCountries(text: string): Parsed<string[]> {
  const value: string[] = [];
  const invalid: string[] = [];
  for (const piece of splitList(text)) {
    if (!isCountryCode(piece)) invalid.push(piece);
    else if (!value.includes(piece.toLowerCase())) value.push(piece.toLowerCase());
  }
  return { value, invalid };
}

/** A whole number 1..65535 written in digits only. */
export function parsePort(text: string): number | null {
  const s = text.trim();
  if (!/^\d{1,5}$/.test(s)) return null;
  const n = Number(s);
  return n >= 1 && n <= 65535 ? n : null;
}

/** "443, 80 80" → 80, 443: sorted, no repeats; wrong pieces are reported. */
export function parsePorts(text: string): Parsed<number[]> {
  const value: number[] = [];
  const invalid: string[] = [];
  for (const piece of splitList(text)) {
    const n = parsePort(piece);
    if (n === null) invalid.push(piece);
    else if (!value.includes(n)) value.push(n);
  }
  return { value: value.sort((a, b) => a - b), invalid };
}

/** The exit of `tor_start`: the pieces of the field as backend wants them, "de, nl". */
export function exitArgument(text: string): Parsed<string> {
  const { value, invalid } = parseCountries(text);
  return { value: value.join(', '), invalid };
}

export type UpstreamKind = 'off' | 'socks5' | 'https';

/** The form: every field as the user sees it, text as text. */
export interface TorForm {
  bridgeMode: BridgeMode;
  bridgeBuiltin: string;
  bridgeLines: string;
  upstreamKind: UpstreamKind;
  upstreamHost: string;
  upstreamPort: string;
  upstreamUser: string;
  upstreamPassword: string;
  ports: string;
  exclude: string;
  strict: boolean;
  startWithApp: boolean;
  idleMinutes: string;
  externalOn: boolean;
  externalPort: string;
  extraTorrc: string;
}

/** The port the "Port for other programs" field offers, the one Tor Browser uses. */
export const SUGGESTED_EXTERNAL_PORT = 9150;

export function formFromSettings(s: TorSettings): TorForm {
  return {
    bridgeMode: s.bridges.mode,
    bridgeBuiltin: s.bridges.builtin,
    bridgeLines: joinLines(s.bridges.lines),
    upstreamKind: s.upstream ? s.upstream.kind : 'off',
    upstreamHost: s.upstream?.host ?? '',
    upstreamPort: s.upstream ? String(s.upstream.port) : '',
    upstreamUser: s.upstream?.username ?? '',
    upstreamPassword: s.upstream?.password ?? '',
    ports: joinList(s.reachable_ports),
    exclude: joinList(s.exclude_countries),
    strict: s.strict_exclude,
    startWithApp: s.start_with_app,
    idleMinutes: String(s.idle_minutes),
    externalOn: s.external_socks_port !== null,
    externalPort: String(s.external_socks_port ?? SUGGESTED_EXTERNAL_PORT),
    extraTorrc: joinLines(s.extra_torrc),
  };
}

/** The fields a check can name; each has its own message under the field. */
export type FormField = 'bridgeLines' | 'upstreamHost' | 'upstreamPort' | 'ports' | 'exclude' | 'idleMinutes' | 'externalPort';

/** A message of the dictionary and what fills it. */
export type FormProblem = {
  key: 'tor_err_countries' | 'tor_err_ports' | 'tor_err_port' | 'tor_err_host' | 'tor_err_idle' | 'tor_err_bridge_lines';
  vars?: Record<string, string>;
};

export type FormResult =
  | { ok: true; settings: TorSettings }
  | { ok: false; problems: Partial<Record<FormField, FormProblem>> };

export const MAX_IDLE_MINUTES = 1440;

export function settingsFromForm(f: TorForm): FormResult {
  const problems: Partial<Record<FormField, FormProblem>> = {};

  const exclude = parseCountries(f.exclude);
  if (exclude.invalid.length) problems.exclude = { key: 'tor_err_countries', vars: { list: exclude.invalid.join(', ') } };

  const ports = parsePorts(f.ports);
  if (ports.invalid.length) problems.ports = { key: 'tor_err_ports', vars: { list: ports.invalid.join(', ') } };

  const bridgeLines = splitLines(f.bridgeLines);
  if (f.bridgeMode === 'custom' && bridgeLines.length === 0) problems.bridgeLines = { key: 'tor_err_bridge_lines' };

  let upstream: TorSettings['upstream'] = null;
  if (f.upstreamKind !== 'off') {
    const host = f.upstreamHost.trim();
    const port = parsePort(f.upstreamPort);
    if (!host || /\s/.test(host)) problems.upstreamHost = { key: 'tor_err_host' };
    if (port === null) problems.upstreamPort = { key: 'tor_err_port' };
    if (host && port !== null && !problems.upstreamHost) {
      upstream = { kind: f.upstreamKind, host, port, username: f.upstreamUser, password: f.upstreamPassword };
    }
  }

  const idleText = f.idleMinutes.trim();
  const idle = /^\d{1,4}$/.test(idleText) ? Number(idleText) : NaN;
  if (!(idle >= 1 && idle <= MAX_IDLE_MINUTES)) {
    problems.idleMinutes = { key: 'tor_err_idle', vars: { max: String(MAX_IDLE_MINUTES) } };
  }

  let external: number | null = null;
  if (f.externalOn) {
    external = parsePort(f.externalPort);
    if (external === null) problems.externalPort = { key: 'tor_err_port' };
  }

  if (Object.keys(problems).length) return { ok: false, problems };

  return {
    ok: true,
    settings: {
      bridges: { mode: f.bridgeMode, builtin: f.bridgeBuiltin, lines: bridgeLines },
      upstream,
      reachable_ports: ports.value,
      exclude_countries: exclude.value,
      strict_exclude: f.strict,
      start_with_app: f.startWithApp,
      idle_minutes: idle,
      external_socks_port: external,
      extra_torrc: splitLines(f.extraTorrc),
    },
  };
}

/** True when the form says something other than the stored settings do. */
export function isChanged(f: TorForm, stored: TorSettings): boolean {
  return JSON.stringify(f) !== JSON.stringify(formFromSettings(stored));
}

/** Error codes of the backend and the dictionary key of each one's message. */
export const ERROR_KEYS = {
  tor_not_installed: 'tor_err_not_installed',
  tor_invalid_country: 'tor_err_invalid_country',
  tor_too_many_instances: 'tor_err_too_many_instances',
  tor_bootstrap_timeout: 'tor_err_bootstrap_timeout',
  tor_failed: 'tor_err_failed',
  tor_stopped: 'tor_err_stopped',
  tor_unknown_instance: 'tor_err_unknown_instance',
  tor_in_use: 'tor_err_in_use',
  tor_no_builtin_bridges: 'tor_err_no_builtin_bridges',
  tor_no_transports: 'tor_err_no_transports',
} as const;

export type TorErrorCode = keyof typeof ERROR_KEYS | 'tor_invalid_settings';

/** The code a backend message starts with (`tor_in_use: …`), and what follows it. */
export function parseTorError(message: string): { code: TorErrorCode | null; detail: string } {
  const m = /^\s*(tor_[a-z_]+)\b[\s:.-]*([\s\S]*)$/.exec(message);
  if (!m) return { code: null, detail: message };
  const code = m[1];
  if (code === 'tor_invalid_settings' || code in ERROR_KEYS) return { code: code as TorErrorCode, detail: m[2].trim() };
  return { code: null, detail: message };
}

/**
 * The text to show for a backend error: our translation of a known code
 * (`translate` gets the dictionary key), the backend's own text for invalid
 * settings and for anything unknown.
 */
export function torErrorText(message: string, translate: (key: (typeof ERROR_KEYS)[keyof typeof ERROR_KEYS]) => string): string {
  const { code, detail } = parseTorError(message);
  if (code === null) return message;
  if (code === 'tor_invalid_settings') return detail || message;
  return translate(ERROR_KEYS[code]);
}

/** The label of an instance: its countries, or null for the one without any. */
export function exitLabel(exit: readonly string[]): string | null {
  return exit.length ? exit.map((c) => c.toUpperCase()).join(', ') : null;
}

/** `127.0.0.1:9050`, the address programs use to reach an instance. */
export function socksAddress(port: number | null): string | null {
  return port === null ? null : `127.0.0.1:${port}`;
}

/** Instances that are up or on their way: failed and stopping ones are not counted. */
export function runningCount(list: readonly { state: string }[]): number {
  return list.filter((i) => i.state === 'starting' || i.state === 'ready' || i.state === 'restarting').length;
}
