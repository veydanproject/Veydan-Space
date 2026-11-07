// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { explainError, explainTorMessage, torErrorCode } from './tor-error';

const tr = (key: string) => `<${key}>`;

describe('the Tor codes in an error message', () => {
  it('finds a code at the start and after other text', () => {
    expect(torErrorCode('tor_not_installed: Tor is not installed')).toBe('tor_not_installed');
    expect(torErrorCode('Proxy error: tor_bootstrap_timeout: gave up after 180 s')).toBe('tor_bootstrap_timeout');
  });

  it('knows every code of the backend', () => {
    for (const code of ['tor_not_installed', 'tor_invalid_country', 'tor_too_many_instances', 'tor_bootstrap_timeout', 'tor_failed', 'tor_stopped', 'tor_in_use', 'tor_no_builtin_bridges', 'tor_no_transports']) {
      expect(torErrorCode(`${code}: text`)).toBe(code);
    }
  });

  it('ignores an unknown code, a code without the colon and a longer word', () => {
    expect(torErrorCode('tor_other: text')).toBeNull();
    expect(torErrorCode('tor_failed without a colon')).toBeNull();
    expect(torErrorCode('autor_failed: text')).toBeNull();
    expect(torErrorCode('')).toBeNull();
    expect(torErrorCode(null)).toBeNull();
  });

  it('translates a Tor message and leaves another one alone', () => {
    expect(explainTorMessage('tor_failed: boom', tr)).toBe('<proxy_tor_err_failed>');
    expect(explainTorMessage('connection refused', tr)).toBe('connection refused');
  });

  it('reads the message of an AppError', () => {
    expect(explainError({ code: 'proxy', message: 'x tor_in_use: busy' }, tr)).toBe('<proxy_tor_err_in_use>');
    expect(explainError('plain', tr)).toBe('plain');
  });
});
