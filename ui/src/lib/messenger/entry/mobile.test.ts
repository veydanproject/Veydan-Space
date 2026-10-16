// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { module } from './mobile';

const bar = (path: string) => module.bar?.(new URL(path, 'http://app'));

describe('the bar of the messenger on a phone', () => {
  it('is the tabs on the tabs: chats, groups, calls, then the settings of the app', () => {
    for (const path of ['/messenger', '/messenger/', '/messenger/groups', '/messenger/calls']) {
      expect(bar(path)?.map((i) => i.id)).toEqual(['chats', 'groups', 'calls', 'settings']);
    }
    expect(bar('/messenger')?.at(-1)?.href).toBe('/settings');
  });

  it('gives the other screens of the module the whole height, and leaves the rest to the shell', () => {
    for (const path of ['/messenger/chat', '/messenger/contacts', '/messenger/settings', '/messenger/call']) expect(bar(path)).toBeNull();
    expect(bar('/notes')).toBeUndefined();
  });
});
