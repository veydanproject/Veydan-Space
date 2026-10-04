// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The messenger's commands of the desktop palette (`ModuleDef.palette`): new
// chat, new group, contacts, settings, and a chat by its name. The module
// gives them as data; the page answers the request it finds here.

import { get } from 'svelte/store';
import { goto } from '$app/navigation';
import { t } from '$lib/core/i18n';
import type { PaletteCommand } from '$lib/core/module';
import { messengerStore } from './store.svelte';
import { chatStore } from './chats/chatStore.svelte';
import { openChat } from './content/actions';
import { BASE } from './mobile/routes';

export type PaletteRequest = 'newChat' | 'newGroup' | 'contacts' | 'settings';

/** What the palette asked the messenger page to show; the page takes it and clears it. */
export const paletteUi = $state<{ request: PaletteRequest | null }>({ request: null });

function request(kind: PaletteRequest) {
  paletteUi.request = kind;
  void goto(BASE);
}

export function messengerPalette(): PaletteCommand[] {
  if (!messengerStore.visible || messengerStore.needsOnboarding) return [];
  const tr = get(t);
  const group = tr('nav_messenger');
  return [
    { id: 'messenger.new-chat', title: tr('msg_newchat_title'), keywords: 'new chat message write', icon: 'message-circle-plus', group, run: () => request('newChat') },
    { id: 'messenger.new-group', title: tr('msg_group_new_title'), keywords: 'new group', icon: 'users-plus', group, run: () => request('newGroup') },
    { id: 'messenger.contacts', title: tr('msg_contacts_title'), keywords: 'contacts people', icon: 'book-user', group, run: () => request('contacts') },
    { id: 'messenger.settings', title: tr('msg_settings_title_full'), keywords: 'chat messenger settings relays', icon: 'user-cog', group, run: () => request('settings') },
    ...chatStore.chats
      .filter((c) => !c.archived)
      .map((c) => ({
        id: `messenger.chat.${c.id}`,
        title: tr('msg_cmd_open_chat', { name: c.title }),
        icon: c.kind === 'group' ? 'users' : 'message-circle',
        group,
        entity: true,
        run: async () => {
          await goto(BASE);
          await openChat(c.id);
        },
      })),
  ];
}
