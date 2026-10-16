// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { beforeEach, describe, expect, it, vi } from 'vitest';

const { openUrl } = vi.hoisted(() => ({ openUrl: vi.fn() }));
vi.mock('$lib/core/api', () => ({ api: { system: { openUrl } } }));

import { ABUSE_MAIL, LEGAL, LEGAL_BASE, openLegal } from './legal';

describe('the legal pages', () => {
  beforeEach(() => openUrl.mockReset());

  it('all live under the site address', () => {
    for (const url of Object.values(LEGAL)) expect(url.startsWith(`${LEGAL_BASE}/`)).toBe(true);
    expect(ABUSE_MAIL.startsWith('mailto:')).toBe(true);
  });

  it('opens an address through the shell', async () => {
    openUrl.mockResolvedValue(undefined);
    await openLegal(LEGAL.terms);
    expect(openUrl).toHaveBeenCalledWith(LEGAL.terms);
  });

  it('logs a failure and does not throw', async () => {
    const log = vi.spyOn(console, 'error').mockImplementation(() => {});
    openUrl.mockRejectedValueOnce(new Error('no opener'));
    await expect(openLegal(LEGAL.privacy)).resolves.toBeUndefined();
    expect(log).toHaveBeenCalled();
    log.mockRestore();
  });
});
