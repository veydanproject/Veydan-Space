// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What a module that requires the messenger shows of it on its own pages
// (platform-spec 6.3: such a module imports the messenger's seams alone).
// The channels of Communities (internal/communities-stage1.md §6.2, §6.3)
// are drawn with the messenger's pieces, not copies of them: the avatar,
// the words of a message with its links, the composer, the media viewer,
// the panel's head, the confirmation and the report; they name people as
// the messenger does, write to an author in its chat, share a link into
// a chat, and keep the pin, the mute and the archive of a row by its chat
// id. Everything here is the messenger's own and stays where it is; this
// file only hands it on.

import { get } from 'svelte/store';
import { t } from '$lib/core/i18n';
import { chatStore } from '../chats/chatStore.svelte';
import { openChat } from '../content/actions';
import { messengerStore } from '../store.svelte';
import { contactLabel, isTauriHost, messengerApi, type MessengerContact, type MessengerProfile } from '../api';

export { default as Avatar } from '../contacts/Avatar.svelte';
export { default as MessageContent } from '../content/MessageContent.svelte';
export { default as ShareDialog } from '../content/ShareDialog.svelte';
export { default as PanelHead } from '../shared/PanelHead.svelte';
export { default as WithInfo } from '../shared/WithInfo.svelte';
export { default as Composer } from '../dm/Composer.svelte';
export { default as AttachButton } from '../media/AttachButton.svelte';
export { default as MediaViewer } from '../media/MediaViewer.svelte';
/** Where the transfer of a file is and what can be done with it, as a chat shows it (`act`: the module's own actions). */
export { default as TransferStatus } from '../media/TransferStatus.svelte';
export type { TransferAction } from '../media/transferLabel';
export type { MessengerTransferProgress as TransferProgress } from '../api';
/** A frame of a video the composer sends, as a chat's composer takes it. */
export { videoPoster } from '../media/poster';
/** Sound and video play from `blob:`, never from `data:` (the page's `media-src`). */
export { playableUrl } from '../media/blob-url';
/** A voice message and a video circle as a chat plays them: the player with its outline and speed, the circle's first frame, one sound at a time. */
export { default as VoicePlayer } from '../media/VoicePlayer.svelte';
export { NO_POSTER, showFirstFrame } from '../media/blob-url';
export { playback } from '../media/playback.svelte';
export { clockOf } from '../media/capture';
/** What a file is called where its name says little (a voice message, a circle), as a chat says it. */
export { mediaTitle } from '../push/wording';
/** A voice message or a circle as the composer hands it over (`onrecording`). */
export type { MessengerRecording as Recording } from '../api';
export { default as BioView } from '../contacts/BioView.svelte';
export { default as SocialLinks } from '../contacts/SocialLinks.svelte';
export { default as CopyField } from '../identity/CopyField.svelte';
export { viewer, type ViewerItem } from '../media/viewer.svelte';
export { confirmStore } from '../shared/confirm.svelte';
export { reportStore, type ReportTarget } from '../shared/report.svelte';
export { longpress } from '../shared/longpress';
export { onPhone } from '../shared/phone';
/** The height of the on-screen keyboard over the page, as the composer follows it. */
export { onKeyboard } from '../shared/keyboard';
/** The marks of the composer's format menu on the selected part of a field (Markdown as it is typed). */
export { applyMark, markOfKey, type Mark } from '../dm/format';
export { clock, dayLabel, listStamp, stamp } from '../shared/time';
export { bytes } from '../shared/format';
/** How the pictures of an album stand in rows, as a chat's album draws them. */
export { mosaic, rowHeight } from '../content/mosaic';
export { contactHaystacks, matchesQuery } from '../shared/search';
export { nameStore, shortKey } from '../groups/names.svelte';
export { default as ServerPicker } from '../servers/ServerPicker.svelte';
export { nameOfServer, serverStore } from '../servers/serverStore.svelte';
export type { MessengerContact, MessengerProfile };
/** The blocks of a Markdown text, as MessageContent takes them (`rich`). */
export type { MdBlock } from '../generated/markdown';
export { contactLabel, isTauriHost };

/** Who I am here: my hex key, `null` before the account is made. */
export const myKey = (): string | null => messengerStore.identity?.pubkey ?? null;

/**
 * The word for the servers of the account where a private server of mine
 * may be chosen instead (ServerPicker's first choice), as «Новая группа» says it.
 */
export const homeServersLabel = (): string =>
  get(t)(messengerStore.manifest?.mode === 'own' ? 'msg_srv_mode' : 'msg_srv_veydan_title');

/** My contacts, as the messenger keeps them. */
export const contacts = (): MessengerContact[] => messengerStore.contacts;

/** The chat with a person, opened and shown wherever the messenger shows a chat. */
export async function writeTo(pubkey: string): Promise<void> {
  const chat = await chatStore.openPeer(pubkey);
  await openChat(chat.id);
}

/** The choices of a row the messenger keeps by its chat id; the list of chats is read again after each. */
export const chatPrefs = {
  setPinned: (chatId: string, pinned: boolean) => chatStore.setPinned(chatId, pinned),
  setMuted: (chatId: string, muted: boolean) => chatStore.setMuted(chatId, muted),
  setArchived: (chatId: string, archived: boolean) => chatStore.setArchived(chatId, archived),
};

/**
 * The servers of the account a new channel starts with: the relays of the
 * home set that write, and the blob hosts that are on (the channel's
 * `relays[]` and `storage[]`, NewChannelInput).
 */
export async function accountServers(): Promise<{ relays: string[]; storage: string[] }> {
  const [relays, media] = await Promise.all([
    messengerApi.relays.list().catch(() => []),
    messengerApi.media.servers().catch(() => []),
  ]);
  return {
    relays: relays.filter((r) => r.enabled && r.write).map((r) => r.url),
    storage: media.filter((m) => m.enabled && m.kind === 'blossom').map((m) => m.public_base || m.url),
  };
}
