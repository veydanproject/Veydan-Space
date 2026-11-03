// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { module } from './mobile';

const bar = (path: string) => module.bar?.(new URL(path, 'http://app'));

describe('the bar of the messenger on a phone', () => {
  it('is the tabs on the tabs: chats, groups, contacts, calls, and no Settings (the avatar of the header leads there)', () => {
    for (const path of ['/messenger', '/messenger/', '/messenger/groups', '/messenger/calls']) {
      expect(bar(path)?.map((i) => i.id)).toEqual(['chats', 'groups', 'contacts', 'calls']);
    }
    // Only the missed calls are red.
    expect(bar('/messenger')?.filter((i) => i.danger).map((i) => i.id)).toEqual(['calls']);
  });

  it('has no apps button in it: the header of each tab carries the logo (MobileFrame hubButton)', () => {
    // Nothing between the tabs: they are the items of the bar, equal columns.
    expect(bar('/messenger')?.every((i) => i.href && i.icon)).toBe(true);
    expect((module as { hubInHeader?: unknown }).hubInHeader).toBeUndefined();
  });

  it('gives the other screens of the module the whole height, and leaves the rest to the shell', () => {
    for (const path of ['/messenger/chat', '/messenger/settings', '/messenger/profile', '/messenger/call']) expect(bar(path)).toBeNull();
    expect(bar('/notes')).toBeUndefined();
  });
});

describe('a section a module gives (the channels)', () => {
  it('is a tab between the groups and the calls, with its list in the bar and its rows on the whole screen', async () => {
    const { provideChatSection } = await import('../seams/sections.svelte');
    const take = provideChatSection({
      id: 'channels', title: 'msg_nav_groups', icon: 'speakerphone', newTitle: 'msg_nav_groups', newIcon: 'plus',
      emptyTitle: 'msg_nav_groups', emptyHint: 'msg_nav_groups', welcomeTitle: 'msg_nav_groups', welcomeText: 'msg_nav_groups',
      rows: () => [], owns: (id) => id.startsWith('channel:'), unread: () => 3, refresh: () => {}, view: (() => {}) as never, create: (() => {}) as never,
    });
    try {
      expect(bar('/messenger')?.map((i) => i.id)).toEqual(['chats', 'groups', 'channels', 'contacts', 'calls']);
      expect(bar('/messenger/section')?.find((i) => i.id === 'channels')?.href).toBe('/messenger/section?id=channels');
      expect(bar('/messenger/section')?.find((i) => i.id === 'channels')?.badge?.()).toBe(3);
      expect(bar('/messenger/item')).toBeNull();
    } finally {
      take();
    }
    expect(bar('/messenger')?.map((i) => i.id)).toEqual(['chats', 'groups', 'contacts', 'calls']);
  });
});
