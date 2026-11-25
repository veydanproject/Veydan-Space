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

describe('a wide window (core/ui/split)', () => {
  const split = (path: string) => module.split?.(new URL(path, 'http://app'));

  it('puts a chat beside the chats, a group beside the groups, by the id, the list itself at its own address', () => {
    expect(split('/messenger')?.home).toBe('/messenger');
    expect(split('/messenger/')?.home).toBe('/messenger');
    expect(split('/messenger/groups')?.home).toBe('/messenger/groups');
    expect(split('/messenger/chat?id=dm%3Aab')?.home).toBe('/messenger');
    expect(split('/messenger/chat?id=group:cd')?.home).toBe('/messenger/groups');
    // One list object for the chats and the chat beside them: the shell keeps it mounted.
    expect(split('/messenger/chat?id=dm:ab')).toBe(split('/messenger'));
    expect(split('/messenger/groups')?.props).toEqual({ kind: 'group' });
  });

  it('leaves the contacts, the calls, the settings and a call as on a phone', () => {
    for (const path of ['/messenger/contacts', '/messenger/calls', '/messenger/settings', '/messenger/call', '/messenger/requests', '/notes']) {
      expect(split(path)).toBeUndefined();
    }
  });

  it('puts a row of a section beside its section, whose address keeps its id', async () => {
    const { provideChatSection } = await import('../seams/sections.svelte');
    const { atHome } = await import('$lib/core/ui/split.svelte');
    expect(split('/messenger/item?id=channel:x')).toBeUndefined();
    const take = provideChatSection({
      id: 'channels', title: 'msg_nav_groups', icon: 'speakerphone', newTitle: 'msg_nav_groups', newIcon: 'plus',
      emptyTitle: 'msg_nav_groups', emptyHint: 'msg_nav_groups', welcomeTitle: 'msg_nav_groups', welcomeText: 'msg_nav_groups',
      rows: () => [], owns: (id) => id.startsWith('channel:'), unread: () => 0, refresh: () => {}, view: (() => {}) as never, create: (() => {}) as never,
    });
    try {
      const def = split('/messenger/item?id=channel:x');
      expect(def?.home).toBe('/messenger/section?id=channels');
      expect(def).toBe(split('/messenger/section?id=channels'));
      expect(def?.props).toEqual({ kind: 'channels' });
      expect(split('/messenger/section?id=other')).toBeUndefined();
      const at = (path: string) => atHome(new URL(path, 'http://app'), def!);
      expect(at('/messenger/section?id=channels')).toBe(true);
      expect(at('/messenger/section?id=other')).toBe(false);
      expect(at('/messenger/item?id=channel:x')).toBe(false);
    } finally {
      take();
    }
  });
});
