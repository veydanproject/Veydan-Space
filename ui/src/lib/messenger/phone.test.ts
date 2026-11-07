// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Which phone the page runs on, by its user agent: an iPad says it is a Mac.

import { describe, expect, it } from 'vitest';
import { phoneOs } from './phone';

describe('phoneOs', () => {
  it('tells Android, an iPhone and an iPad from a computer', () => {
    expect(phoneOs('Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 Chrome/126.0 Mobile Safari/537.36')).toBe('android');
    expect(phoneOs('Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148')).toBe('ios');
    expect(phoneOs('Mozilla/5.0 (iPad; CPU OS 15_8 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148')).toBe('ios');
    const mac = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)';
    expect(phoneOs(mac, 5)).toBe('ios');
    expect(phoneOs(mac, 0)).toBe('other');
    expect(phoneOs('Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko)')).toBe('other');
  });
});
