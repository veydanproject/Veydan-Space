// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { api } from '$lib/core/api';

/**
 * The pages Google Play and the people who use the app read: the privacy policy, the terms,
 * child safety, deleting an account, reporting abuse. The addresses are the site's and change only with it.
 */
export const LEGAL_BASE = 'https://veydan.net/legal';

export const LEGAL = {
  privacy: `${LEGAL_BASE}/privacy`,
  terms: `${LEGAL_BASE}/terms`,
  childSafety: `${LEGAL_BASE}/child-safety`,
  deleteAccount: `${LEGAL_BASE}/delete-account`,
  abuse: `${LEGAL_BASE}/abuse`,
};

/** Where a report of abuse goes when the person prefers mail to the page. */
export const ABUSE_MAIL = 'mailto:abuse@veydan.net';

/** Opens a page or a mail address in the system's browser or mail app; a failure is logged, never thrown, so a tap cannot break the screen. */
export async function openLegal(url: string): Promise<void> {
  await api.system.openUrl(url).catch((e) => console.error('open_url failed', e));
}
