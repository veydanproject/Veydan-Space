// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Who is on the other end of a call, as the chats name them: the title of
// the chat (my own name for a contact first), its picture; for a chat not
// loaded yet, what the names store knows of the key.

import type { CallView } from '../api';
import { chatStore } from '../chats/chatStore.svelte';
import { nameStore } from '../groups/names.svelte';

export interface CallPeer {
  name: string;
  /** A picture's address, for `Avatar`'s `url`. */
  picture: string | null;
  /** Picks the colour of the initials. */
  seed: string;
}

export function callPeer(call: Pick<CallView, 'peer' | 'chat_id'>): CallPeer {
  const chat = chatStore.chats.find((c) => c.id === call.chat_id);
  return {
    name: chat?.title || nameStore.label(call.peer),
    picture: chat?.picture ?? nameStore.picture(call.peer),
    seed: call.peer,
  };
}
