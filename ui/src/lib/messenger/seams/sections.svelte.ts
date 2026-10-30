// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The sections other modules add to the list of chats (platform-spec 6.3:
// a module sees of the messenger its seams alone). The channels of
// Communities are rows of the list under a section of their own
// («Каналы»), beside «Чаты» and «Группы» (internal/communities-stage1.md
// §6.2): the module gives the rows and the page of an open row, the
// messenger keeps the list, the pin, the mute and the archive of each row
// by its chat id (`channel:<id>`). A module given here at its start and
// taken back at its stop leaves no trace: no tab, no rows, no badge.

import type { Component } from 'svelte';
import type { TranslationKey } from '$lib/core/i18n';

/** One row of a section: what the list of chats shows of it. */
export interface SectionRow {
  /** The chat id the messenger keeps the choices of the row by (`channel:<id>`). */
  chatId: string;
  title: string;
  picture: string | null;
  /** What the avatar's colour is made of when there is no picture. */
  seed: string;
  /** The newest thing in it, unix seconds; `null` when nothing. */
  at: number | null;
  /** The line under the title. */
  preview: string;
  unread: number;
  pinned: boolean;
  muted: boolean;
  archived: boolean;
  /** What waits for my answer there (requests to be let in): a count apart from the unread. */
  asks?: number;
}

/** What the page of an open row is given. */
export interface SectionViewProps {
  chatId: string;
  /** Back to the list (the arrow of a phone, of a narrow window). */
  onback: () => void;
  /** Another row of the same section to show instead (a link followed inside). */
  onopen?: (chatId: string) => void;
}

/** What the dialog of a new row is given. */
export interface SectionCreateProps {
  open: boolean;
  /** A link to open on the dialog's tab of links. */
  link?: string;
  onopened?: (chatId: string) => void;
}

/** A section of the list of chats, given by a module. */
export interface ChatSectionHost {
  /** The section's id: the `kind` of the list, the tab on a phone (`channels`). */
  id: string;
  title: TranslationKey;
  icon: string;
  /** The button of a new row: its tooltip and its icon. */
  newTitle: TranslationKey;
  newIcon: string;
  /** The list with no rows: a line and a hint. */
  emptyTitle: TranslationKey;
  emptyHint: TranslationKey;
  /** The pane with no row open (a computer): a line and a hint. */
  welcomeTitle: TranslationKey;
  welcomeText: TranslationKey;
  /** The rows, read reactively. */
  rows: () => SectionRow[];
  /** The rows are not read yet (the module is starting): the list says it loads, not that there are none. */
  loading?: () => boolean;
  /** What the list says while it loads. */
  loadingTitle?: TranslationKey;
  /** Whether a chat id is a row of this section (`channel:…`), before its rows are read. */
  owns: (chatId: string) => boolean;
  /** The badge of the section: what is unread in rows not muted nor archived. */
  unread: () => number;
  /** The rows are read again (a choice of the list changed one). */
  refresh: () => void;
  /** Over the list, under the search (the invitations waiting for an answer). */
  above?: Component;
  /**
   * What `above` holds, for the narrow list of covers (72), where the banner itself is not drawn: its icon
   * and its words as a tooltip; `null` when it holds nothing.
   */
  aboveSummary?: () => { icon: string; words: string } | null;
  /** The page of an open row: a pane on a computer, the screen on a phone. */
  view: Component<SectionViewProps>;
  /** The dialog of a new row. */
  create: Component<SectionCreateProps, object, 'open'>;
  /**
   * The name of what a link of this section leads to, before it is here
   * (a channel's card, asked of its relay); `null` when it cannot be told.
   * A card of a link shows it in place of the section's word.
   */
  linkTitle?: (link: string) => Promise<string | null>;
}

class SectionSeam {
  // Raw: the hosts are compared by identity when given back.
  hosts = $state.raw<ChatSectionHost[]>([]);
  /** The chat ids each module of the product would own, said when it loads, on or off. */
  known = $state.raw<{ id: string; owns: (chatId: string) => boolean }[]>([]);

  /**
   * A chat id of a section the product has but that is not given now: its
   * module is switched off (a link of it says so instead of «unknown»).
   */
  isOff(chatId: string | null | undefined): boolean {
    return !!chatId && !this.ofChat(chatId) && this.known.some((k) => k.owns(chatId));
  }

  get(id: string | null | undefined): ChatSectionHost | null {
    return this.hosts.find((h) => h.id === id) ?? null;
  }

  /** The section a chat id belongs to (`channel:…` → the host of `channels`). */
  ofChat(chatId: string | null | undefined): ChatSectionHost | null {
    if (!chatId) return null;
    return this.hosts.find((h) => h.owns(chatId)) ?? null;
  }
}

export const chatSections = new SectionSeam();

/** A module of the product says, once it loads, which chat ids its section owns, whether it is on or not. */
export function declareChatSection(id: string, owns: (chatId: string) => boolean): void {
  chatSections.known = [...chatSections.known.filter((k) => k.id !== id), { id, owns }];
}

/** The module gives its section; returns the way to take it back. */
export function provideChatSection(host: ChatSectionHost): () => void {
  chatSections.hosts = [...chatSections.hosts.filter((h) => h.id !== host.id), host];
  return () => {
    chatSections.hosts = chatSections.hosts.filter((h) => h !== host);
  };
}
