// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Types of what a message is made of. Two kinds of links exist and they
// are two types: a text becomes one of them only in `tokenize.ts`, and
// what takes one does not take the other.

import type { MessengerMessage } from '../api';

declare const kind: unique symbol;

/** `veydan://…`, `npub1…`, `nostr:npub1…`: leads inside the messenger. Taken apart by the runtime only. */
export type InternalLinkText = string & { readonly [kind]: 'internal' };

/** `http(s)://…`: leads outside. Opened in the system browser only. */
export type ExternalUrl = string & { readonly [kind]: 'external' };

export type Segment =
  | { type: 'text'; text: string }
  | { type: 'external'; text: string; url: ExternalUrl }
  | { type: 'internal'; text: string; link: InternalLinkText };

export type LinkSegment = Exclude<Segment, { type: 'text' }>;

/** What an album holds: pictures and videos, files, or both (pictures on top, files under them). */
export type AlbumVariant = 'visual' | 'files' | 'mixed';

/** A run of messages of one author: where it begins and ends, and what is shown once per run. */
interface Run {
  first: boolean;
  last: boolean;
  /** Chats of many, messages of others: the name, at the first of a run. */
  showAuthor: boolean;
  /** Chats of many, messages of others: the picture, at the last of a run. */
  showAvatar: boolean;
  /** Chats of many, messages of others: room is left for the picture. */
  indent: boolean;
}

export type TimelineItem =
  | { type: 'day'; id: string; at: number }
  /** One line, or several in a row folded into one. */
  | { type: 'system'; id: string; messages: MessengerMessage[] }
  | ({ type: 'bubble'; id: string; message: MessengerMessage } & Run)
  /** Attachments sent by one action, or one picture alone. */
  | ({ type: 'album'; id: string; variant: AlbumVariant; messages: MessengerMessage[] } & Run);
