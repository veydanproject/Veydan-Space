// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Notifications of a computer, the page's side. The app shows them itself
// (Rust decides between the system and the card in the app); the page
// gives them the words of the user's language and opens the chat a click
// asks for.

import { goto } from '$app/navigation';
import { listen } from '@tauri-apps/api/event';
import { get } from 'svelte/store';
import { t } from '$lib/core/i18n';
import { messengerApi, type MessengerNoticeWords } from '../api';
import { openChat } from '../content/actions';
import { BASE } from '../mobile/routes';

export const NOTICE_TAP_EVENT = 'messenger://notice-tap';

function words(): MessengerNoticeWords {
  const tt = get(t);
  // Templates go as they are: Rust puts the number in `{n}` and the name in `{name}`.
  return {
    new_message: tt('msg_notice_new'),
    new_messages: tt('msg_notice_new_many'),
    more: tt('msg_notice_more'),
    request: tt('msg_notice_request'),
    group_invite: tt('msg_notice_group_invite'),
    group_request: tt('msg_notice_group_request'),
    group_welcome: tt('msg_notice_group_welcome'),
    photo: tt('msg_notice_photo'),
    video: tt('msg_notice_video'),
    voice: tt('msg_notice_voice'),
    circle: tt('msg_notice_circle'),
    audio: tt('msg_notice_audio'),
    file: tt('msg_notice_file'),
    album: tt('msg_notice_album'),
    files: tt('msg_notice_files'),
    link_group: tt('msg_notice_link_group'),
    link_group_nameless: tt('msg_notice_link_group_nameless'),
    link_contact: tt('msg_notice_link_contact'),
    link_contact_nameless: tt('msg_notice_link_contact_nameless'),
  };
}

/** The messenger with the chat open, from wherever the user was. */
export async function openFromNotice(chat: string | null) {
  await goto(BASE);
  if (chat) await openChat(chat);
}

/** Runs for as long as the window does; returns what stops it. */
export async function startDesktopNotices(): Promise<() => void> {
  const offWords = t.subscribe(() => {
    messengerApi.desktopNotify.words(words()).catch(() => {});
  });
  const offTap = await listen<{ chat: string | null }>(NOTICE_TAP_EVENT, (e) => {
    openFromNotice(e.payload?.chat ?? null).catch(() => {});
  });
  return () => {
    offWords();
    offTap();
  };
}
