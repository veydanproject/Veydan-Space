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

function units(messages: MessengerMessage[]): Unit[] {
  const out: Unit[] = [];
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
    if (key && prev && (prev.kind === 'bubble' || prev.kind === 'album') && prevLast && sameDay && prev.messages.length < MAX_ALBUM
      && prevLast.sender_pubkey === m.sender_pubkey && albumKey(prevLast) === key) {
      if (prev.kind !== 'album') prev.variant = mediaFamily(prevLast) ?? 'visual';
      prev.kind = 'album';
      if (mediaFamily(m) !== prev.variant) prev.variant = 'mixed';
      prev.messages.push(m);
      continue;
    }
    // A picture alone is shown as a picture, not as a file in a bubble; a reply keeps its quote.
    const alone = mediaFamily(m) === 'visual' && !m.reply_to;
    out.push({ kind: alone ? 'album' : 'bubble', variant: 'visual', messages: [m] });
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
    if (u.kind === 'album') out.push({ type: 'album', id: `album:${head.id}`, variant: u.variant, messages: u.messages, ...run });
    else out.push({ type: 'bubble', id: head.id, message: head, ...run });
  });
  return out;
}
