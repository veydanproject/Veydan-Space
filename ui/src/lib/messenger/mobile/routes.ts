// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Phone routes of the messenger. One place, so a host that mounts the
// module elsewhere changes a single constant.

export const BASE = '/messenger';

/** The tabs of the bottom bar: the chats between two (`BASE`), the groups, the journal of calls. */
export const groupsHref = () => `${BASE}/groups`;
export const callsHref = () => `${BASE}/calls`;

/** A section of the list a module gives (seams/sections: the channels), a tab of the bar. */
export const sectionHref = (id: string) => `${BASE}/section?id=${encodeURIComponent(id)}`;

/** A row of such a section, the whole screen. */
export const sectionItemHref = (chatId: string) => `${BASE}/item?id=${encodeURIComponent(chatId)}`;

/** The contacts: QR codes, adding, the list (reached from «Новый чат»). */
export const contactsHref = () => `${BASE}/contacts`;

export const chatHref = (chatId: string) => `${BASE}/chat?id=${encodeURIComponent(chatId)}`;

/** Where a chat opened from elsewhere is shown: a row of a module's section (`owned`, seams/sections) on its own screen. */
export const openHref = (chatId: string, owned: boolean) => (owned ? sectionItemHref(chatId) : chatHref(chatId));

/** The call under way, the whole screen. */
export const callHref = () => `${BASE}/call`;

/** The room of the group call I am in, the whole screen. */
export const groupCallHref = () => `${BASE}/group-call`;

/** The requests of strangers waiting for an answer. */
export const requestsHref = () => `${BASE}/requests`;

/** One request, by the stranger's key (hex). */
export const requestHref = (peer: string) => `${BASE}/request?peer=${encodeURIComponent(peer)}`;

/** «QR»: my code to show, and the camera for someone else's; `scan` opens it with the camera on. */
export const qrHref = (scan = false) => `${BASE}/qr${scan ? '?scan=1' : ''}`;

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
