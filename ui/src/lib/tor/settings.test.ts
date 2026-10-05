// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import {
  exitArgument, exitLabel, formFromSettings, isChanged, parseCountries, parsePort, parsePorts,
  parseTorError, runningCount, settingsFromForm, socksAddress, splitLines, splitList, torErrorText,
} from './settings';
import type { TorSettings } from './types';

const defaults: TorSettings = {
  bridges: { mode: 'none', builtin: 'obfs4', lines: [] },
  upstream: null,
  reachable_ports: [],
  exclude_countries: [],
  strict_exclude: false,
  start_with_app: false,
  idle_minutes: 10,
  external_socks_port: null,
  extra_torrc: [],
};

describe('lists typed as text', () => {
  it('splits on commas, semicolons and blanks', () => {
    expect(splitList(' de,nl ; fr  ,')).toEqual(['de', 'nl', 'fr']);
    expect(splitList('')).toEqual([]);
    expect(splitLines('a\r\n\n  b  \n')).toEqual(['a', 'b']);
  });

  it('lower-cases countries, drops repeats and reports wrong codes', () => {
    expect(parseCountries('DE, nl de')).toEqual({ value: ['de', 'nl'], invalid: [] });
    expect(parseCountries('de, deu, 1a, é')).toEqual({ value: ['de'], invalid: ['deu', '1a', 'é'] });
    expect(exitArgument('NL,de')).toEqual({ value: 'nl, de', invalid: [] });
    expect(exitArgument('')).toEqual({ value: '', invalid: [] });
  });

  it('accepts ports 1..65535 in digits only', () => {
    expect(parsePort('443')).toBe(443);
    expect(parsePort(' 65535 ')).toBe(65535);
    expect(parsePort('0')).toBeNull();
    expect(parsePort('65536')).toBeNull();
    expect(parsePort('80.5')).toBeNull();
    expect(parsePort('-1')).toBeNull();
    expect(parsePort('')).toBeNull();
    expect(parsePort('1e3')).toBeNull();
  });

  it('sorts ports, drops repeats and reports wrong ones', () => {
    expect(parsePorts('443, 80 80')).toEqual({ value: [80, 443], invalid: [] });
    expect(parsePorts('80, x, 70000')).toEqual({ value: [80], invalid: ['x', '70000'] });
    expect(parsePorts('')).toEqual({ value: [], invalid: [] });
  });
});

describe('the form and the settings', () => {
  it('turns the defaults into a form and back unchanged', () => {
    const form = formFromSettings(defaults);
    expect(form.externalPort).toBe('9150');
    expect(form.externalOn).toBe(false);
    expect(settingsFromForm(form)).toEqual({ ok: true, settings: defaults });
    expect(isChanged(form, defaults)).toBe(false);
  });

  it('round-trips a full set of settings', () => {
    const full: TorSettings = {
      bridges: { mode: 'custom', builtin: 'snowflake', lines: ['obfs4 1.2.3.4:443 AAAA'] },
      upstream: { kind: 'socks5', host: 'proxy.local', port: 1080, username: 'u', password: 'p' },
      reachable_ports: [80, 443],
      exclude_countries: ['ru', 'cn'],
      strict_exclude: true,
      start_with_app: true,
      idle_minutes: 30,
      external_socks_port: 9150,
      extra_torrc: ['AvoidDiskWrites 1'],
    };
    expect(settingsFromForm(formFromSettings(full))).toEqual({ ok: true, settings: full });
  });

  it('notices a change', () => {
    const form = formFromSettings(defaults);
    expect(isChanged({ ...form, strict: true }, defaults)).toBe(true);
  });

  it('names every wrong field', () => {
    const form = {
      ...formFromSettings(defaults),
      bridgeMode: 'custom' as const,
      upstreamKind: 'https' as const,
      upstreamHost: '',
      upstreamPort: '99999',
      ports: '80, x',
      exclude: 'de, usa',
      idleMinutes: '0',
      externalOn: true,
      externalPort: 'abc',
    };
    const r = settingsFromForm(form);
    expect(r.ok).toBe(false);
    if (!r.ok) {
      expect(Object.keys(r.problems).sort()).toEqual(
        ['bridgeLines', 'exclude', 'externalPort', 'idleMinutes', 'ports', 'upstreamHost', 'upstreamPort'],
      );
      expect(r.problems.exclude?.vars?.list).toBe('usa');
    }
  });

  it('ignores the fields of a part that is off', () => {
    const form = { ...formFromSettings(defaults), upstreamHost: '', upstreamPort: 'x', externalPort: 'x' };
    expect(settingsFromForm(form).ok).toBe(true);
  });
});

describe('errors of the backend', () => {
  it('reads the code a message starts with', () => {
    expect(parseTorError('tor_in_use: something uses tor')).toEqual({ code: 'tor_in_use', detail: 'something uses tor' });
    expect(parseTorError('tor_in_use').code).toBe('tor_in_use');
    expect(parseTorError('boom').code).toBeNull();
    expect(parseTorError('tor_something_new: x').code).toBeNull();
  });

  it('translates a known code, shows the backend text for invalid settings and for the unknown', () => {
    const tr = (key: string) => `[${key}]`;
    expect(torErrorText('tor_in_use: busy', tr)).toBe('[tor_err_in_use]');
    expect(torErrorText('tor_invalid_settings: port 0 is not allowed', tr)).toBe('port 0 is not allowed');
    expect(torErrorText('disk full', tr)).toBe('disk full');
  });
});

describe('lines about instances', () => {
  it('labels the exit and the address', () => {
    expect(exitLabel([])).toBeNull();
    expect(exitLabel(['de', 'nl'])).toBe('DE, NL');
    expect(socksAddress(9050)).toBe('127.0.0.1:9050');
    expect(socksAddress(null)).toBeNull();
  });

  it('counts the instances that are up or coming up', () => {
    expect(runningCount([{ state: 'ready' }, { state: 'starting' }, { state: 'failed' }, { state: 'stopping' }, { state: 'restarting' }])).toBe(3);
    expect(runningCount([])).toBe(0);
  });
});
