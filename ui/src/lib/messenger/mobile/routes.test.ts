// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { BASE, callHref, chatHref, shouldFollow } from './routes';

const at = (url: string) => {
  const u = new URL(url, 'http://app');
  return { pathname: u.pathname, search: u.search };
};

const A = `dm:${'a'.repeat(64)}`;
const B = `group:${'b'.repeat(64)}`;

describe('shouldFollow', () => {
  it('does not go again to the chat on the screen', () => {
    expect(shouldFollow(chatHref(A), at(chatHref(A)))).toBe(false);
    // The same chat with its id written out.
    expect(shouldFollow(chatHref(A), at(`${BASE}/chat?id=${A}`))).toBe(false);
    expect(shouldFollow(BASE, at(BASE))).toBe(false);
  });

  it('goes to another chat, to the list, from a call', () => {
    expect(shouldFollow(chatHref(B), at(chatHref(A)))).toBe(true);
    expect(shouldFollow(BASE, at(chatHref(A)))).toBe(true);
    expect(shouldFollow(chatHref(A), at(BASE))).toBe(true);
    expect(shouldFollow(chatHref(A), at(callHref()))).toBe(true);
  });
});
