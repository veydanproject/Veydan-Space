// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The channels as a section of the messenger's list of chats, «Каналы»
// (internal/communities-stage1.md §6.2): the rows are `channels_list`,
// each under its chat id `channel:<id>`, whose pin, mute and archive the
// messenger keeps; a row opens the feed of the channel; the new one is
// made, or come into by its link, in the dialog of the module. Given to
// the messenger's seam at the module's start, taken back at its stop.

import { get } from 'svelte/store';
import { t } from '$lib/core/i18n';
import { declareChatSection, provideChatSection, type ChatSectionHost, type SectionRow } from '$lib/messenger/seams/sections.svelte';
import ChannelFeed from './ChannelFeed.svelte';
import InvitesBar from './InvitesBar.svelte';
import NewChannelDialog from './NewChannelDialog.svelte';
import { channelStore } from './channelStore.svelte';
import { nameStore } from './names.svelte';
import { coverStore } from './coverStore.svelte';
import { previewLine } from './lines';
import { channelsApi, type ChannelView } from './api';

const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);

/** A channel as a row of the list of chats. */
export function rowOf(c: ChannelView, translate = tr): SectionRow {
  return {
    chatId: c.chat_id || `channel:${c.id}`,
    title: c.name,
    picture: coverStore.src(c.id, c.picture),
    seed: c.id,
    // The date of the last post (none yet: when the channel came here), never of an opening or a read.
    at: c.last_at,
    preview: previewLine(c, translate),
    unread: c.membership === 'joined' ? c.unread : 0,
    pinned: c.pinned,
    muted: c.muted,
    archived: c.archived,
    asks: channelStore.requests[c.id] ?? 0,
  };
}

/** The names of the channels of links asked once per link: a chat may show the same link many times. */
const cardNames = new Map<string, Promise<string | null>>();

/** The name a link's channel has on its signed card; `null` when its relay does not answer. */
function linkTitle(link: string): Promise<string | null> {
  let name = cardNames.get(link);
  if (!name) {
    // A relay that did not answer is asked again by the next card that shows the link.
    name = channelsApi.previewLink(link).then((p) => p.name || null).catch(() => { cardNames.delete(link); return null; });
    cardNames.set(link, name);
  }
  return name;
}

export const channelSection: ChatSectionHost = {
  id: 'channels',
  title: 'chan_nav',
  icon: 'speakerphone',
  newTitle: 'chan_new_title',
  newIcon: 'plus',
  emptyTitle: 'chan_list_empty',
  emptyHint: 'chan_list_empty_hint',
  welcomeTitle: 'chan_welcome_title',
  welcomeText: 'chan_welcome_text',
  rows: () => {
    void get(t);
    // A disbanded channel stays, said so, until it is forgotten: what was read is still there.
    return channelStore.list.map((c) => rowOf(c));
  },
  // Until the module answers (it opens the channels after the page starts), «loading», not «no channels».
  loading: () => !channelStore.loaded,
  loadingTitle: 'chan_list_loading',
  owns: (chatId) => chatId.startsWith('channel:'),
  unread: () => channelStore.unread,
  refresh: () => channelStore.scheduleList(),
  above: InvitesBar,
  // The same invitations, as the narrow list shows them: an icon, their names and who invites (InvitesBar's words).
  aboveSummary: () => {
    const list = channelStore.invites;
    if (!list.length) return null;
    const tr = get(t);
    return { icon: 'user-plus', words: list.map((i) => `${i.name} · ${tr('chan_invited_by', { name: nameStore.label(i.peer) })}`).join('\n') };
  },
  view: ChannelFeed,
  create: NewChannelDialog,
  linkTitle,
};

// Said at load, on or off: a link of a channel in a chat then knows the channels are only switched off.
declareChatSection(channelSection.id, channelSection.owns);

let taken: (() => void) | null = null;

/** The section is in the list of chats, and the channels are read and listened to. */
export async function startChannels(): Promise<void> {
  taken ??= provideChatSection(channelSection);
  await channelStore.start().catch(() => {});
}

/** The module is off: no section, no rows, nothing listened to. */
export function stopChannels(): void {
  taken?.();
  taken = null;
  channelStore.stop();
  cardNames.clear();
}
