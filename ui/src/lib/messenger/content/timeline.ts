// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Messages of a chat as what is drawn: days, runs of one author, albums,
// folded system lines, the lines of calls. One function for every chat there is.

import type { MessengerMessage } from '../api';
import { dayKey } from '../shared/time';
import { isCallLine } from '../calls/words';
import type { AlbumVariant, TimelineItem } from './types';

/** A pause longer than this starts a new run. */
export const RUN_GAP_SECS = 300;
/** This many system lines in a row become one. */
export const SYSTEM_FOLD = 3;
/** As many as one album holds; more sent at once make several. */
export const MAX_ALBUM = 10;

export interface TimelineOptions {
  /** A chat of many: names and pictures of authors are shown. */
  many: boolean;
}

interface Unit {
  kind: 'system' | 'call' | 'bubble' | 'album';
  variant: AlbumVariant;
  messages: MessengerMessage[];
}

/** What an attachment is shown as in an album: a picture, or a file card; `null` for anything never put in one. */
export function mediaFamily(m: MessengerMessage): 'visual' | 'files' | null {
  if (m.content_type !== 'media' || m.deleted || !m.media) return null;
  const { kind } = m.media as { kind?: unknown };
  if (kind === 'image' || kind === 'video') return 'visual';
  if (kind === 'file' || kind === 'audio') return 'files';
  return null;
}

/** What says that messages were sent by one action; `null` when nothing does. */
export function albumKey(m: MessengerMessage): string | null {
  const family = mediaFamily(m);
  const { batch } = (m.media ?? {}) as { batch?: unknown };
  return family && typeof batch === 'string' && batch ? batch : null;
}

/** The place of a file among those picked with it (`pos`), when its sender said it. */
function posOf(m: MessengerMessage): number | null {
  const { pos } = (m.media ?? {}) as { pos?: unknown };
  return typeof pos === 'number' && Number.isInteger(pos) && pos >= 0 ? pos : null;
}

/**
 * The files of an album in the order they were picked: each comes when its
 * upload ends, a video after the photos picked behind it. Only when every
 * one says its place; otherwise as they came.
 */
function picked(messages: MessengerMessage[]): MessengerMessage[] {
  if (messages.length < 2 || messages.some((m) => posOf(m) === null)) return messages;
  return [...messages].sort((a, b) => posOf(a)! - posOf(b)!);
}

function units(messages: MessengerMessage[]): Unit[] {
  const out: Unit[] = [];
  // Where the files of a batch gather: each takes the time its upload ended,
  // so what came in between (a message of anyone) stands after the album, not in it.
  const gathering = new Map<string, Unit>();
  for (const m of messages) {
    const prev = out[out.length - 1];
    const prevLast = prev?.messages[prev.messages.length - 1];
    const sameDay = !!prevLast && dayKey(prevLast.created_at) === dayKey(m.created_at);

    // A call stands alone: a missed one folded among others would not be seen.
    if (isCallLine(m)) {
      out.push({ kind: 'call', variant: 'visual', messages: [m] });
      continue;
    }
    if (m.content_type === 'system') {
      if (prev?.kind === 'system' && sameDay) prev.messages.push(m);
      else out.push({ kind: 'system', variant: 'visual', messages: [m] });
      continue;
    }
    const key = albumKey(m);
    const into = key ? gathering.get(key) : undefined;
    const intoLast = into?.messages[into.messages.length - 1];
    if (into && intoLast && into.messages.length < MAX_ALBUM && intoLast.sender_pubkey === m.sender_pubkey
      && dayKey(intoLast.created_at) === dayKey(m.created_at)) {
      if (into.kind !== 'album') into.variant = mediaFamily(intoLast) ?? 'visual';
      into.kind = 'album';
      if (mediaFamily(m) !== into.variant) into.variant = 'mixed';
      into.messages.push(m);
      continue;
    }
    // A picture alone is shown as a picture, not as a file in a bubble; a reply keeps its quote.
    const alone = mediaFamily(m) === 'visual' && !m.reply_to;
    const unit: Unit = { kind: alone ? 'album' : 'bubble', variant: 'visual', messages: [m] };
    out.push(unit);
    if (key) gathering.set(key, unit);
  }
  return out;
}

/** Does a run go on from `a` to `b`? */
function continues(a: Unit | undefined, b: Unit | undefined): boolean {
  if (!a || !b || a.kind === 'system' || b.kind === 'system' || a.kind === 'call' || b.kind === 'call') return false;
  const from = a.messages[a.messages.length - 1];
  const to = b.messages[0];
  return from.sender_pubkey === to.sender_pubkey
    && dayKey(from.created_at) === dayKey(to.created_at)
    && to.created_at - from.created_at <= RUN_GAP_SECS;
}

export function buildTimeline(messages: MessengerMessage[], opts: TimelineOptions): TimelineItem[] {
  const list = units(messages);
  const out: TimelineItem[] = [];
  let day = '';

  list.forEach((u, i) => {
    const head = u.messages[0];
    const key = dayKey(head.created_at);
    if (key !== day) {
      day = key;
      out.push({ type: 'day', id: `day:${key}`, at: head.created_at });
    }

    if (u.kind === 'call') {
      out.push({ type: 'call', id: `call:${head.id}`, message: head });
      return;
    }
    if (u.kind === 'system') {
      if (u.messages.length >= SYSTEM_FOLD) out.push({ type: 'system', id: `sys:${head.id}`, messages: u.messages });
      else for (const m of u.messages) out.push({ type: 'system', id: `sys:${m.id}`, messages: [m] });
      return;
    }

    const first = !continues(list[i - 1], u);
    const last = !continues(u, list[i + 1]);
    const others = opts.many && head.direction === 'in';
    const run = { first, last, showAuthor: others && first, showAvatar: others && last, indent: others };
    // The id stays with the file that came first: a later one placed before it does not draw the album anew.
    if (u.kind === 'album') out.push({ type: 'album', id: `album:${head.id}`, variant: u.variant, messages: picked(u.messages), ...run });
    else out.push({ type: 'bubble', id: head.id, message: head, ...run });
  });
  return out;
}
