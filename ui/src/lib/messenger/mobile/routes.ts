// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Phone routes of the messenger. One place, so a host that mounts the
// module elsewhere changes a single constant.

export const BASE = '/messenger';

export const chatHref = (chatId: string) => `${BASE}/chat?id=${encodeURIComponent(chatId)}`;

/** The call under way, the whole screen. */
export const callHref = () => `${BASE}/call`;

/** The room of the group call I am in, the whole screen. */
export const groupCallHref = () => `${BASE}/group-call`;

/**
 * Whether a tapped notification has somewhere to go from `here`. The page
 * it leads to may be open already (a second tap on the same chat): going
 * there again would only add the same page to the history.
 */
export function shouldFollow(route: string, here: { pathname: string; search: string }): boolean {
  const to = new URL(route, 'http://app');
  const at = new URL(here.pathname + here.search, 'http://app');
  // Compared as parameters: `dm:…` and `dm%3A…` are the same chat.
  return to.pathname !== at.pathname || to.searchParams.toString() !== at.searchParams.toString();
}
