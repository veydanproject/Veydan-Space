// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What a message is, in the user's words. The app says what came in facts
// (`messenger_core::Body`); the phone words them in its own strings, a
// computer's notification in the templates this page gives it, and the
// card in the app here. The three say the same.

import type { TranslationKey } from '$lib/core/i18n';

/** `messenger_core::Body`: what is absent is left out. */
export type NoticeBody =
  | { t: 'text'; text: string }
  | { t: 'link'; link: 'web' | 'group' | 'contact'; title: string }
  | { t: 'media'; kind: string; name: string; caption?: string; duration_ms?: number; batch?: string }
  | { t: 'invite' | 'join_request' | 'welcome'; group_name: string };

type Tr = (key: TranslationKey, vars?: Record<string, string>) => string;

const EMOJI: Record<string, string> = {
  image: '📷',
  video: '🎬',
  voice: '🎤',
  circle: '⭕',
  audio: '🎵',
  file: '📎',
};

const WORD: Record<string, TranslationKey> = {
  image: 'msg_notice_photo',
  video: 'msg_notice_video',
  voice: 'msg_notice_voice',
  circle: 'msg_notice_circle',
  audio: 'msg_notice_audio',
  file: 'msg_notice_file',
};

/** `12400` → `0:12`; an hour and more as `1:02:03`. */
export function duration(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  const two = (n: number) => String(n).padStart(2, '0');
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return h > 0 ? `${h}:${two(m)}:${two(s % 60)}` : `${m}:${two(s % 60)}`;
}

/** Files are told by their name; a picture, a video, a recording by what it is. */
function named(kind: string): boolean {
  return kind === 'file' || kind === 'audio';
}

/** One line of one message. `null`: nothing may be said of it. */
export function bodyLine(body: NoticeBody | null, t: Tr): string {
  if (!body) return t('msg_notice_new');
  switch (body.t) {
    case 'text':
      return body.text;
    case 'link':
      if (body.link === 'group') {
        return `🔗 ${body.title ? t('msg_notice_link_group', { name: body.title }) : t('msg_notice_link_group_nameless')}`;
      }
      if (body.link === 'contact') {
        return `👤 ${body.title ? t('msg_notice_link_contact', { name: body.title }) : t('msg_notice_link_contact_nameless')}`;
      }
      return `🔗 ${body.title}`;
    case 'media': {
      const kind = EMOJI[body.kind] ? body.kind : 'file';
      const word = t(WORD[kind]);
      if (named(kind)) {
        const name = body.name || word;
        return `${EMOJI[kind]} ${body.caption ? `${name} · ${body.caption}` : name}`;
      }
      if (body.caption) return `${EMOJI[kind]} ${body.caption}`;
      return `${EMOJI[kind]} ${word}${body.duration_ms ? ` (${duration(body.duration_ms)})` : ''}`;
    }
    case 'invite':
      return t('msg_notice_group_invite');
    case 'join_request':
      return t('msg_notice_group_request');
    case 'welcome':
      return t('msg_notice_group_welcome');
  }
}

/** Files sent together, as far as they came: an album, or a few documents. */
export interface Album {
  batch: string;
  files: boolean;
  n: number;
  caption?: string;
}

/** The album a message belongs to, if it was sent with others. */
export function albumOf(body: NoticeBody | null): Album | null {
  if (body?.t !== 'media' || !body.batch) return null;
  if (body.kind === 'voice' || body.kind === 'circle') return null;
  return { batch: body.batch, files: named(body.kind), n: 1, caption: body.caption };
}

/**
 * `next` joins `last` when it is one more of the same batch: the line
 * then says how many, not which. Null: it does not.
 */
export function joinAlbum(last: Album | null | undefined, next: Album | null): Album | null {
  if (!last || !next || last.batch !== next.batch) return null;
  return { ...last, files: last.files && next.files, n: last.n + next.n, caption: last.caption ?? next.caption };
}

export function albumLine(album: Album, t: Tr): string {
  const head = album.files
    ? `📎 ${t('msg_notice_files', { n: String(album.n) })}`
    : `🖼 ${t('msg_notice_album', { n: String(album.n) })}`;
  return album.caption ? `${head} · ${album.caption}` : head;
}

/** What the quote of a message may say of it (`MessengerReplyPreview`). */
export interface QuoteOf {
  text: string | null;
  deleted?: boolean;
  media?: Record<string, unknown> | null;
}

/**
 * The small picture of a quoted photo or video, as an image source, or null.
 * Only a JPEG preview that came with the media (base64, "/9j/…") is shown;
 * a message taken back has none.
 */
export function replyThumb(quote: QuoteOf): string | null {
  if (quote.deleted) return null;
  const m = quote.media;
  if (!m || !['image', 'video', 'circle'].includes(m.kind as string)) return null;
  const thumb = m.thumb;
  return typeof thumb === 'string' && thumb.startsWith('/9j/') ? `data:image/jpeg;base64,${thumb}` : null;
}

/**
 * The line under a reply: the quoted message in a few words. A message
 * taken back is told first; a photo with no caption is a photo, never
 * "deleted"; what cannot be told is left empty.
 */
export function replyLine(quote: QuoteOf, cardLabel: string | null, t: Tr): string {
  if (quote.deleted) return t('msg_message_deleted');
  if (quote.text) return quote.text;
  if (cardLabel) return `👤 ${cardLabel}`;
  const m = quote.media;
  if (m && typeof m.kind === 'string') {
    return bodyLine(
      {
        t: 'media',
        kind: m.kind,
        name: typeof m.name === 'string' ? m.name : '',
        duration_ms: typeof m.duration_ms === 'number' ? m.duration_ms : undefined,
      },
      t,
    );
  }
  return '';
}
