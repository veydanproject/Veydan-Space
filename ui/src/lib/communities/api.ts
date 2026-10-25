// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The commands of the module Communities (`channel_*`, crates/communities;
// internal/communities-stage1.md §6.1), the event of its runtime, and the
// refusals it answers with. In a plain browser (the preview, the harness of
// screenshots) every command has an answer of the demo below, so each page
// can be seen in each of its states without the app.

import { call, mockCommand } from '$lib/core/api';
import type {
  ChannelAction,
  ChannelDetail,
  ChannelPreview,
  ChannelView,
  FeedCursorView,
  FeedItemView,
  FeedPageView,
  InviteView,
  NewChannelInput,
  RequestView,
  ScheduleView,
  StaffView,
  StoragePlanView,
  TransferView,
  UploadView,
  ImportedFile,
} from './generated/channels';
import type { InviteStatusView, KeeperStatus, LogEntry, LogPage, ModAction, ModSettings, QueueItem, QueuePage, ReadersPage, TicketsView } from './generated/moderation';
import type { MdBlock } from '$lib/messenger/seams/shared';
import type { FeedItemViewLv, LevelInput, LevelRequestView, LevelView, NewScopeInput, ScopeSetInput, ScopeView, StaffViewLv } from './levels/types';

export type * from './generated/channels';
export type * from './levels/types';

/** How words are written: `md` for Markdown; plain is `null`, as the messenger's. */
export type TextFormat = 'md';

/**
 * The blocks of a demo text written in Markdown, enough for the preview and
 * the post editor: headings, quotes, lists, fences, rules, paragraphs whose
 * lines break, and `**bold**`, `*italic*`, `~~struck~~`, `\`code\``, links.
 */
function demoRich(text: string): MdBlock[] {
  const plain = { bold: false, italic: false, strike: false };
  const inlines = (line: string): MdInline[] =>
    line.split(/(\*\*[^*]+\*\*|~~[^~]+~~|\*[^*\s][^*]*\*|`[^`]+`|\[[^\]]*\]\([^)]*\))/).filter(Boolean).map((p): MdInline => {
      if (p.startsWith('**')) return { kind: 'text', text: p.slice(2, -2), style: { ...plain, bold: true } };
      if (p.startsWith('~~')) return { kind: 'text', text: p.slice(2, -2), style: { ...plain, strike: true } };
      if (p.startsWith('*')) return { kind: 'text', text: p.slice(1, -1), style: { ...plain, italic: true } };
      if (p.startsWith('`')) return { kind: 'code', text: p.slice(1, -1) };
      const link = p.match(/^\[([^\]]*)\]\(([^)]*)\)$/);
      if (link) return { kind: 'link', url: link[2], host: link[2].replace(/^https?:\/\//, '').split('/')[0] || null, children: [{ kind: 'text', text: link[1] || link[2] }] };
      return { kind: 'text', text: p };
    });
  const lines = (block: string): MdInline[] => block.split('\n').flatMap((l, i) => [...(i ? [{ kind: 'break' } as MdInline] : []), ...inlines(l)]);
  const blocks: MdBlock[] = [];
  const src = text.replace(/\r\n?/g, '\n');
  // A fence keeps its blank lines: cut the text into fences and the rest first.
  for (const [i, part] of src.split(/^```[^\n]*\n?/m).entries()) {
    if (i % 2 === 1) { blocks.push({ kind: 'code', lang: null, text: part.replace(/\n$/, '') }); continue; }
    // A list, a quote or a heading starts a block of its own after a line of words, as in Markdown.
    const kind = (l: string) => (/^#{1,6}\s/.test(l) ? `h${l}` : l.startsWith('>') ? 'q' : /^[-*+]\s/.test(l) ? 'ul' : /^\d+[.)]\s/.test(l) ? 'ol' : 'p');
    const runs = part.split(/\n{2,}/).flatMap((b) => b.split('\n').filter((l) => l.trim()).reduce<string[][]>((out, l) => {
      const last = out[out.length - 1];
      if (last && kind(last[0]) === kind(l) && !kind(l).startsWith('h')) last.push(l); else out.push([l]);
      return out;
    }, []));
    for (const ls of runs) {
      const block = ls.join('\n');
      if (/^([-*_])(\s*\1){2,}\s*$/.test(block)) blocks.push({ kind: 'rule' });
      else if (/^#{1,6}\s/.test(block) && ls.length === 1) blocks.push({ kind: 'heading', level: block.match(/^#+/)![0].length, inlines: inlines(block.replace(/^#+\s+/, '')) });
      else if (ls.every((l) => l.startsWith('>'))) blocks.push({ kind: 'quote', blocks: demoRich(ls.map((l) => l.replace(/^>\s?/, '')).join('\n')) });
      else if (ls.every((l) => /^[-*+]\s/.test(l))) blocks.push({ kind: 'list', ordered: false, start: 1, items: ls.map((l) => ({ task: null, blocks: [{ kind: 'para', inlines: inlines(l.slice(2)) }] })) });
      else if (ls.every((l) => /^\d+[.)]\s/.test(l))) blocks.push({ kind: 'list', ordered: true, start: Number(ls[0].match(/^\d+/)![0]), items: ls.map((l) => ({ task: null, blocks: [{ kind: 'para', inlines: inlines(l.replace(/^\d+[.)]\s+/, '')) }] })) });
      else blocks.push({ kind: 'para', inlines: lines(block) });
    }
  }
  return blocks;
}
type MdInline = Extract<MdBlock, { kind: 'para' }>['inlines'][number];
export type * from './generated/moderation';

/** The event of the module's runtime: `{ name, payload }`. */
export const CHANNEL_EVENT = 'communities://event';

export interface ChannelUiEvent {
  name: string;
  payload: unknown;
}

export const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** The roles of a channel, lowest first; a reader has none. */
/** A frame of a video the composer took (base64 JPEG) and the size of the video, as the messenger's poster. */
export interface ChannelPoster {
  jpeg: string;
  width: number;
  height: number;
}

export type ChannelRole = 'member' | 'author' | 'moderator' | 'admin' | 'owner';

/** Membership as `ChannelView.membership` says it. */
export type Membership = 'none' | 'joining' | 'requested' | 'joined' | 'rejected' | 'out' | 'disbanded';

/** What a file of a post carries (the messenger's envelope `t: media`), as far as a page reads it. */
export interface PostMedia {
  kind: 'image' | 'video' | 'audio' | 'file' | 'voice' | 'circle';
  name: string;
  mime: string;
  size: number;
  caption?: string;
  dim?: [number, number];
  /** A small JPEG, base64. */
  thumb?: string;
  /** The hash of the whole file (hex): the same file is fetched once. */
  sha256?: string;
  /** Files sent by one action (an album), as the messenger's envelope names them. */
  batch?: string;
  /** The length of a voice message or a circle (or of a sound, a video), ms. */
  duration_ms?: number;
  /** The outline of a voice message: up to 64 values, 0..255. */
  waveform?: number[];
}

/** A voice message or a video circle as the composer hands it over (the messenger's `onrecording`). */
export interface ChannelRecording {
  kind: 'voice' | 'circle';
  mime: string;
  duration_ms: number;
  waveform?: number[];
  blob: Blob;
}

/** A recording made in the app: a voice message or a circle, drawn by their players, never a file line. */
export const isRecording = (m: Pick<PostMedia, 'kind'> | null | undefined): m is PostMedia & { kind: 'voice' | 'circle' } =>
  m?.kind === 'voice' || m?.kind === 'circle';

/** The bytes of a recording as base64, as a chat's go (mobile IPC has no raw bodies). */
async function blobToBase64(blob: Blob): Promise<string> {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let bin = '';
  const step = 0x8000;
  for (let i = 0; i < bytes.length; i += step) bin += String.fromCharCode(...bytes.subarray(i, i + step));
  return btoa(bin);
}

/** The file of an item; `null` for words alone or an envelope this build cannot read. */
export function mediaOf(item: Pick<FeedItemView, 'media_json'>): PostMedia | null {
  if (!item.media_json) return null;
  try {
    const v = JSON.parse(item.media_json) as Partial<PostMedia> & { t?: string };
    if (!v || typeof v !== 'object' || typeof v.name !== 'string') return null;
    return {
      kind: (['image', 'video', 'audio', 'file', 'voice', 'circle'] as const).find((k) => k === v.kind) ?? 'file',
      name: v.name,
      mime: typeof v.mime === 'string' ? v.mime : '',
      size: typeof v.size === 'number' ? v.size : 0,
      caption: typeof v.caption === 'string' ? v.caption : undefined,
      dim: Array.isArray(v.dim) && v.dim.length === 2 ? [Number(v.dim[0]), Number(v.dim[1])] : undefined,
      thumb: typeof v.thumb === 'string' ? v.thumb : undefined,
      sha256: typeof v.sha256 === 'string' && v.sha256 ? v.sha256 : undefined,
      batch: typeof v.batch === 'string' && v.batch ? v.batch : undefined,
      duration_ms: typeof v.duration_ms === 'number' && v.duration_ms > 0 ? v.duration_ms : undefined,
      waveform: Array.isArray(v.waveform) ? v.waveform.filter((x): x is number => typeof x === 'number').slice(0, 64) : undefined,
    };
  } catch {
    return null;
  }
}

// ── Refusals ────────────────────────────────────────────────────────────

/** The text of anything thrown: an error of a command is `{ code, message }`. */
export function errorText(e: unknown): string {
  if (e != null && typeof e === 'object' && 'message' in e) return String((e as { message: unknown }).message ?? '');
  return typeof e === 'string' ? e : e instanceof Error ? e.message : String(e);
}

/** The code of an error of a command (`other`, `vault_locked`, …). */
export function errorCode(e: unknown): string | null {
  return e != null && typeof e === 'object' && 'code' in e ? String((e as { code: unknown }).code) : null;
}

/** The refusal of the module a text starts with (`channel_<code>: …`); `null` when it is not one. */
export function channelErrorCode(e: unknown): string | null {
  const m = /^(channel_[a-z_]+):/.exec(errorText(e).trim());
  return m ? m[1] : null;
}

// ── The commands ────────────────────────────────────────────────────────

export const channelsApi = {
  list: () => call<ChannelView[]>('channels_list'),
  get: (communityId: string) => call<ChannelDetail>('channel_get', { communityId }),
  create: (input: NewChannelInput) => call<ChannelDetail>('channel_create', { input }),
  /** Where the files of a new channel of mine live: the free public media of the messenger without Veydan Cloud (14 days). */
  storagePlan: () => call<StoragePlanView>('channel_storage_plan'),
  act: (communityId: string, action: ChannelAction) => call<ChannelDetail>('channel_act', { communityId, action }),
  openLink: (link: string, note?: string) => call<ChannelDetail>('channel_open_link', { link, note: note || null }),
  /** The card of a link's channel, before joining it; `channel_preview_unreachable` when its relay does not answer. */
  previewLink: (link: string) => call<ChannelPreview>('channel_preview_link', { link }),
  invite: (communityId: string, who: string) => call<InviteView>('channel_invite', { communityId, who }),
  invites: (direction: 'in' | 'out') => call<InviteView[]>('channel_invites', { direction }),
  answerInvite: (inviteId: string, accept: boolean) => call<void>('channel_answer_invite', { inviteId, accept }),
  answerRequest: (communityId: string, requester: string, approve: boolean) =>
    call<ChannelDetail>('channel_answer_request', { communityId, requester, approve }),
  avatarSet: (communityId: string, src: string) => call<ChannelDetail>('channel_avatar_set', { communityId, src }),
  avatarRemove: (communityId: string) => call<ChannelDetail>('channel_avatar_remove', { communityId }),
  rotateLink: (communityId: string) => call<ChannelDetail>('channel_rotate_link', { communityId }),
  linkQr: (communityId: string) => call<string>('channel_link_qr', { communityId }),
  forget: (communityId: string) => call<void>('channel_forget', { communityId }),

  /**
   * `next`: the cursor the page before answered with (`FeedPageView.next`), passed back as it came.
   * `scopeId`: the posts of one section (communities-wire §14.3); none, of all I am in.
   */
  history: (communityId: string, next?: FeedCursorView, limit = 50, scopeId: string | null = null) =>
    call<FeedPageView>('channel_history', { communityId, limit, next: next ?? null, scopeId }),
  thread: (communityId: string, postId: string, next?: FeedCursorView, limit = 50) =>
    call<FeedPageView>('channel_thread', { communityId, postId, limit, next: next ?? null }),
  /** `fmt`: `md` when the words are Markdown (the composer's format menu), as a chat's text. `scopeId`: its section; none, everyone's. */
  post: (communityId: string, text: string, fmt: TextFormat | null = null, scopeId: string | null = null) =>
    call<string>('channel_post', { communityId, text, fmt, scopeId }),
  comment: (communityId: string, postId: string, text: string, parent?: string | null, fmt: TextFormat | null = null) =>
    call<string>('channel_comment', { communityId, postId, parent: parent ?? null, text, fmt }),
  react: (communityId: string, target: string, emoji: string, set: boolean) =>
    call<string>('channel_react', { communityId, target, emoji, set }),
  /** An edit says how its words are written: plain unless `fmt`. */
  edit: (communityId: string, target: string, text: string, fmt: TextFormat | null = null) =>
    call<string>('channel_edit', { communityId, target, text, fmt }),
  /** The blocks of words written in Markdown as the feed will show them (the post editor's preview); `null`: shown as plain text. Nothing is sent. */
  mdPreview: (text: string) => call<MdBlock[] | null>('channel_md_preview', { text }),
  remove: (communityId: string, target: string) => call<string>('channel_delete', { communityId, target }),
  pin: (communityId: string, postId: string, pinned: boolean) => call<void>('channel_pin', { communityId, postId, pinned }),
  read: (communityId: string, at?: number) => call<void>('channel_read', { communityId, at: at ?? null }),
  /**
   * What of mine the outbox of a channel gave up on (`channel.outbox`
   * `failed`) tried again now, all of it; each row tells of itself as
   * `channel.outbox`. The page of the channel as it is after.
   */
  outboxRetry: (communityId: string) => call<ChannelDetail>('channel_outbox_retry', { communityId }),

  /** `scopeId`: the section of a new post with a file; a comment's is its post's. */
  /** `batch`: the files picked together (an album), the same for each; `caption`: with the last of them; `fmt`: `md` when the caption is Markdown. */
  /**
   * `original`: a photo goes as it is, not made smaller; `poster`: the frame of a video the page took,
   * the readers' preview before they fetch it (as a chat's file).
   */
  mediaSend: (
    communityId: string,
    src: string,
    opts: { postId?: string; parent?: string | null; caption?: string; fmt?: TextFormat | null; scopeId?: string | null; batch?: string; original?: boolean; poster?: ChannelPoster | null } = {},
  ) =>
    call<UploadView>('channel_media_send', {
      communityId, src, postId: opts.postId ?? null, parent: opts.parent ?? null, caption: opts.caption || null, fmt: opts.caption ? (opts.fmt ?? null) : null, scopeId: opts.scopeId ?? null,
      batch: opts.batch ?? null, options: { original: opts.original ?? false, poster: opts.poster ?? null },
    }),
  /**
   * A voice message or a circle recorded in the composer, as a post (`scopeId`: its section) or a comment
   * under `postId` (`parent`: the comment it answers): its row at once, its progress as `channel.media`,
   * as a picked file's. A reader who may comment records a comment.
   */
  mediaRecord: async (communityId: string, rec: ChannelRecording, opts: { postId?: string; parent?: string | null; scopeId?: string | null } = {}) =>
    call<UploadView>('channel_media_record', {
      communityId,
      recording: {
        kind: rec.kind, mime: rec.mime, duration_ms: Math.round(rec.duration_ms),
        waveform: rec.waveform?.length ? rec.waveform.slice(0, 64).map((x) => Math.max(0, Math.min(255, Math.round(x)))) : null,
        data_base64: await blobToBase64(rec.blob),
      },
      postId: opts.postId ?? null, parent: opts.parent ?? null, scopeId: opts.scopeId ?? null,
    }),
  mediaFetch: (communityId: string, messageId: string) => call<string>('channel_media_fetch', { communityId, messageId }),
  /** The fetched file as a `data:` URL to show inline; `null` for a type a page does not show. */
  mediaUrl: (communityId: string, messageId: string) => call<string | null>('channel_media_url', { communityId, messageId }),
  /** The cover of a channel as a `data:` URL, read from the hosts of the channel; `null` without one. */
  cover: (communityId: string) => call<string | null>('channel_cover', { communityId }),
  /** A picked file as the tray shows it: a picture as a small square `data:` URL; `null` for anything else. */
  pickedPreview: (src: string) => call<string | null>('channel_picked_preview', { src }),
  mediaCancel: (uploadId: string) => call<void>('channel_media_cancel', { uploadId }),
  /** A failed or paused file sent again, on from the chunks the hosts hold; its row is `uploading` at once, its progress comes as `channel.media`. */
  mediaRetry: (uploadId: string) => call<UploadView>('channel_media_retry', { uploadId }),
  /** Pauses a file of mine on its way (upload id) or the file of a message coming down (message id); whether one ran. */
  mediaPause: (id: string) => call<boolean>('channel_media_pause', { id }),
  /** Gives up the file of a message coming down; whether one ran. */
  mediaFetchCancel: (messageId: string) => call<boolean>('channel_media_fetch_cancel', { messageId }),
  /** "Retry now" of a file waiting for its next attempt (upload id or message id); whether one ran. */
  mediaWake: (id: string) => call<boolean>('channel_media_wake', { id }),
  /** Where a fetched video or sound plays, whatever its size (the messenger's server of files); `null`: take `mediaUrl`. */
  mediaPlay: (communityId: string, messageId: string) => call<string | null>('channel_media_play', { communityId, messageId }),
  /** A picked file made a file of the device, with where the page reads a video for its frame. */
  mediaImport: (src: string) => call<ImportedFile>('channel_media_import', { src }),
  uploads: (communityId: string) => call<UploadView[]>('channel_uploads', { communityId }),

  moderation: {
    /** One card per author; with `author`, each of their messages. */
    queue: (communityId: string, before?: number, limit = 50, author?: string) =>
      call<QueuePage>('channel_moderation_queue', { communityId, author: author ?? null, before: before ?? null, limit }),
    mod: (communityId: string, action: ModAction) => call<void>('channel_moderation_mod', { communityId, action }),
    log: (communityId: string, before?: number, limit = 50) =>
      call<LogPage>('channel_moderation_log', { communityId, before: before ?? null, limit }),
    settings: (communityId: string) => call<ModSettings>('channel_moderation_settings', { communityId }),
    settingsSet: (communityId: string, settings: ModSettings) =>
      call<ModSettings>('channel_moderation_settings_set', { communityId, settings }),
    inherit: (communityId: string, from: string) => call<number>('channel_moderation_inherit', { communityId, from }),
    inviteCreate: (communityId: string, who: string) => call<InviteView>('channel_moderation_invite_create', { communityId, who }),
    inviteList: (communityId: string) => call<InviteView[]>('channel_moderation_invite_list', { communityId }),
    inviteRevoke: (inviteId: string) => call<void>('channel_moderation_invite_revoke', { inviteId }),
    inviteStatus: (communityId: string) => call<InviteStatusView>('channel_moderation_invite_status', { communityId }),
    blocked: (communityId: string) => call<string[]>('channel_moderation_blocked', { communityId }),
    requests: (communityId: string) => call<RequestView[]>('channel_moderation_requests', { communityId }),
    requestAct: (communityId: string, requester: string, approve: boolean) =>
      call<void>('channel_moderation_request_act', { communityId, requester, approve }),
    requestsCount: (communityId: string) => call<number>('channel_moderation_requests_count', { communityId }),
    /** The readers of the channel, for the team only: newest first, `query` a start of the hex key; `partial` while shards of the snapshot are on their way. */
    readers: (communityId: string, offset = 0, limit = 100, query?: string) =>
      call<ReadersPage>('channel_readers', { communityId, query: query ?? null, offset, limit }),
  },

  /** Levels and sections (communities-wire §14): the owner names the levels and gives them sections and rights. */
  levels: {
    scopes: (communityId: string) => call<ScopeView[]>('channel_scopes', { communityId }),
    scopeAdd: (communityId: string, input: NewScopeInput) => call<ChannelDetail>('channel_scope_add', { communityId, input }),
    scopeDel: (communityId: string, scopeId: string) => call<ChannelDetail>('channel_scope_del', { communityId, scopeId }),
    /** The title, the look or the hiding of a section (the owner, §14.7); a field left out stays. */
    scopeSet: (communityId: string, scopeId: string, input: ScopeSetInput) => call<ChannelDetail>('channel_scope_set', { communityId, scopeId, input }),
    /** The sections in this order (the owner). */
    scopesOrder: (communityId: string, ids: string[]) => call<ChannelDetail>('channel_scopes_order', { communityId, ids }),
    /** A picked picture as the photo of a section (a square, as a cover). */
    scopePhotoSet: (communityId: string, scopeId: string, src: string) => call<ChannelDetail>('channel_scope_photo_set', { communityId, scopeId, src }),
    /** The photo of a section as a `data:` URL; `null`: none, or no host has it now. */
    scopePhoto: (communityId: string, scopeId: string) => call<string | null>('channel_scope_photo', { communityId, scopeId }),
    list:(communityId: string) => call<LevelView[]>('channel_levels', { communityId }),
    /** A level made (no `id`) or changed; the levels as they are after it. */
    set: (communityId: string, level: LevelInput) => call<LevelView[]>('channel_level_set', { communityId, level }),
    del: (communityId: string, id: number) => call<LevelView[]>('channel_level_del', { communityId, id }),
    /** The levels of a person; the base level is always among them. */
    setFor: (communityId: string, who: string, levels: number[]) => call<ChannelDetail>('channel_set_levels', { communityId, who, levels }),
    /** A reader asks the moderators for a level. */
    request: (communityId: string, level: number, note?: string) => call<void>('channel_level_request', { communityId, level, note: note || null }),
    requests: (communityId: string) => call<LevelRequestView[]>('channel_level_requests', { communityId }),
    /** A moderator gives the level (`set_levels` in the journal) or refuses it, with a reason. */
    requestAct: (communityId: string, requester: string, level: number, grant: boolean, reason?: string) =>
      call<void>('channel_level_request_act', { communityId, requester, level, grant, reason: reason || null }),
  },

  /** The tickets of a moderator (communities-wire §17): those held, and a batch asked of the keeper. */
  tickets: {
    held: (communityId: string) => call<TicketsView>('channel_tickets', { communityId }),
    request: (communityId: string, n: number) => call<TicketsView>('channel_tickets_request', { communityId, n }),
  },

  /** The channel handed to a new owner (communities-wire §16): offered by the owner, answered by the one named. */
  transfer: {
    offer: (communityId: string, to: string) => call<ChannelDetail>('channel_transfer', { communityId, to }),
    answer: (communityId: string, accept: boolean) => call<ChannelDetail>('channel_transfer_answer', { communityId, accept }),
  },

  keeper: {
    status: (communityId: string) => call<KeeperStatus>('channel_keeper_status', { communityId }),
    rotateNow: (communityId: string) => call<KeeperStatus>('channel_rotate_now', { communityId }),
    /** The owner's switch «Держать хранителя включённым» of a channel kept on this phone. */
    keepOn: (communityId: string, on: boolean) => call<KeeperStatus>('channel_keeper_keep_on', { communityId, on }),
    /** «Сделать хранителем это устройство» (wire §15); `emergency`: the device that keeps it is lost. */
    take: (communityId: string, emergency = false) => call<KeeperStatus>('channel_keeper_take', { communityId, emergency }),
  },
};

/** Listens to the module's runtime; the way to stop. Nothing comes in a plain browser but what the demo tells. */
export async function listenChannels(fn: (ev: ChannelUiEvent) => void): Promise<() => void> {
  demoListeners.add(fn);
  let off: (() => void) | null = null;
  if (isTauri) {
    const { listen } = await import('@tauri-apps/api/event');
    off = await listen<ChannelUiEvent>(CHANNEL_EVENT, (e) => fn(e.payload));
  }
  return () => {
    demoListeners.delete(fn);
    off?.();
  };
}

// ── The demo of a plain browser ─────────────────────────────────────────
// A channel of each state a page shows: mine, kept here, with a queue,
// requests and a file that did not go; one of another owner where my key
// is old; one that waits for the keeper's key, and one whose keeper is
// away; a request sent; one I was removed from; one disbanded. People are
// those of the messenger's demo.

const demoListeners = new Set<(ev: ChannelUiEvent) => void>();
const emit = (name: string, payload: unknown) => queueMicrotask(() => demoListeners.forEach((f) => f({ name, payload })));

const hex = (c: string) => c.repeat(64).slice(0, 64);
const ME = 'ab'.repeat(32);
const ALICE = hex('1a');
const BORIS = hex('2b');
const MARINA = hex('9a');
const NIKITA = hex('8e');
const VERA = hex('3c');
const now = () => Math.floor(Date.now() / 1000);
const T0 = now();
const H = 3600;
const D = 24 * H;

/**
 * A picture for the demo: a gradient with a word. A PNG drawn on a canvas
 * where there is one (the messenger's Avatar shows raster pictures only),
 * an SVG of its own elsewhere.
 */
function picture(a: string, b: string, word = ''): string {
  if (typeof document !== 'undefined') {
    try {
      const c = document.createElement('canvas');
      c.width = 480; c.height = 320;
      const g = c.getContext('2d');
      if (g) {
        const grad = g.createLinearGradient(0, 0, 480, 320);
        grad.addColorStop(0, a); grad.addColorStop(1, b);
        g.fillStyle = grad; g.fillRect(0, 0, 480, 320);
        g.fillStyle = 'rgba(255,255,255,.35)'; g.beginPath(); g.arc(360, 90, 45, 0, Math.PI * 2); g.fill();
        g.fillStyle = 'rgba(0,0,0,.25)'; g.beginPath(); g.moveTo(0, 260); g.lineTo(130, 160); g.lineTo(235, 250); g.lineTo(320, 190); g.lineTo(480, 300); g.lineTo(480, 320); g.lineTo(0, 320); g.fill();
        g.fillStyle = 'rgba(255,255,255,.9)'; g.font = '600 120px sans-serif'; g.textAlign = 'center'; g.textBaseline = 'middle';
        if (word.length <= 2) g.fillText(word, 240, 160); else { g.font = '600 28px sans-serif'; g.textAlign = 'left'; g.fillText(word, 24, 48); }
        return c.toDataURL('image/png');
      }
    } catch { /* the SVG below */ }
  }
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="960" height="640" viewBox="0 0 960 640"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${a}"/><stop offset="1" stop-color="${b}"/></linearGradient></defs><rect width="960" height="640" fill="url(#g)"/><circle cx="720" cy="180" r="90" fill="#fff" fill-opacity=".35"/><path d="M0 520 L260 320 L470 500 L640 380 L960 600 L960 640 L0 640Z" fill="#000" fill-opacity=".25"/><text x="48" y="96" font-family="sans-serif" font-size="56" fill="#fff" fill-opacity=".9">${word}</text></svg>`;
  return `data:image/svg+xml;base64,${btoa(unescape(encodeURIComponent(svg)))}`;
}

const media = (name: string, kind: PostMedia['kind'], size: number, caption?: string, dim?: [number, number], batch?: string) =>
  JSON.stringify({ v: 1, t: 'media', kind, name, mime: kind === 'image' ? 'image/jpeg' : 'application/pdf', size, caption, dim, batch });

/** A voice message or a circle of the demo, as the messenger's envelope carries it. */
const recMedia = (kind: 'voice' | 'circle', size: number, ms: number, waveform?: number[]) =>
  JSON.stringify({ v: 1, t: 'media', kind, name: kind === 'voice' ? 'voice.weba' : 'circle.webm', mime: kind === 'voice' ? 'audio/webm' : 'video/webm', size, duration_ms: ms, waveform });

/** An outline of a voice message the demo draws: loud and quiet in turns. */
const demoWave = (seed: number) => Array.from({ length: 48 }, (_, i) => Math.round(60 + 150 * Math.abs(Math.sin((i + seed) * 0.45)) * (0.5 + 0.5 * Math.abs(Math.cos((i + seed) * 0.13)))));

/** What the demo's recordings play: the bytes recorded in the page, else a tone or a moving circle made here. */
const demoRecordings = new Map<string, string>();

/** A tone of `ms` (a WAV `data:` URL) for a voice message of the demo that was not recorded in the page. */
function demoTone(ms: number): string {
  const rate = 8000;
  const n = Math.max(1, Math.round((rate * Math.min(ms, 8000)) / 1000));
  const buf = new DataView(new ArrayBuffer(44 + n * 2));
  const str = (o: number, s: string) => [...s].forEach((c, i) => buf.setUint8(o + i, c.charCodeAt(0)));
  str(0, 'RIFF'); buf.setUint32(4, 36 + n * 2, true); str(8, 'WAVE'); str(12, 'fmt ');
  buf.setUint32(16, 16, true); buf.setUint16(20, 1, true); buf.setUint16(22, 1, true); buf.setUint32(24, rate, true);
  buf.setUint32(28, rate * 2, true); buf.setUint16(32, 2, true); buf.setUint16(34, 16, true); str(36, 'data'); buf.setUint32(40, n * 2, true);
  for (let i = 0; i < n; i++) buf.setInt16(44 + i * 2, Math.round(Math.sin((2 * Math.PI * 330 * i) / rate) * 6000 * Math.abs(Math.sin(i / 900))), true);
  let bin = '';
  const bytes = new Uint8Array(buf.buffer);
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return `data:audio/wav;base64,${btoa(bin)}`;
}

/** A short round video of the demo (a `data:` URL), recorded from a drawn canvas; `null` where the page cannot record. */
let demoCircleVideo: Promise<string | null> | null = null;
function demoCircle(): Promise<string | null> {
  demoCircleVideo ??= new Promise((done) => {
    try {
      const c = document.createElement('canvas');
      c.width = c.height = 240;
      const g = c.getContext('2d');
      const stream = (c as HTMLCanvasElement & { captureStream?: (fps: number) => MediaStream }).captureStream?.(25);
      if (!g || !stream || typeof MediaRecorder === 'undefined') { done(null); return; }
      const rec = new MediaRecorder(stream, { mimeType: 'video/webm' });
      const parts: Blob[] = [];
      rec.ondataavailable = (e) => { if (e.data.size) parts.push(e.data); };
      rec.onstop = () => {
        const r = new FileReader();
        r.onload = () => done(typeof r.result === 'string' ? r.result : null);
        r.onerror = () => done(null);
        r.readAsDataURL(new Blob(parts, { type: 'video/webm' }));
      };
      let f = 0;
      const draw = () => {
        const grad = g.createLinearGradient(0, 0, 240, 240);
        grad.addColorStop(0, '#5b6cff'); grad.addColorStop(1, '#1fc8a9');
        g.fillStyle = grad; g.fillRect(0, 0, 240, 240);
        g.fillStyle = 'rgba(255,255,255,.85)';
        g.beginPath(); g.arc(120, 104 + Math.sin(f / 6) * 6, 46, 0, Math.PI * 2); g.fill();
        g.beginPath(); g.ellipse(120, 236, 90, 70, 0, Math.PI, 0); g.fill();
        f++;
      };
      draw();
      const timer = setInterval(draw, 40);
      rec.start();
      setTimeout(() => { clearInterval(timer); rec.stop(); }, 1600);
    } catch { done(null); }
  });
  return demoCircleVideo;
}

/** A demo file on its way, with the album it goes in, or the recording it is. */
type DemoUpload = UploadView & { batch?: string; fmt?: string; rec?: { kind: 'voice' | 'circle'; ms: number; waveform?: number[]; data: string } };

interface DemoChannel {
  view: ChannelView;
  history: string;
  post_mode: string;
  comments: string;
  premod: string;
  schedule?: ScheduleView;
  pinned: string[];
  staff: StaffViewLv[];
  has_key: boolean;
  waiting: number;
  link?: string;
  posts: FeedItemView[];
  comments_of: Record<string, FeedItemView[]>;
  queue: QueueItem[];
  log: LogEntry[];
  requests: RequestView[];
  banned: string[];
  /** Levels and sections (communities-wire §14); a channel without them has everyone's section alone. */
  scopes?: ScopeView[];
  levels?: LevelView[];
  my_levels?: number[];
  level_requests?: LevelRequestView[];
  /** The keeper on its way between the owner's devices (wire §15). */
  keeper_move?: { taking?: 'handover' | 'emergency'; since?: number; handing?: boolean };
  /** A handover of the channel to a new owner (wire §16). */
  transfer?: TransferView;
  /** Tickets I hold as a moderator (wire §17). */
  tickets?: number;
  /** Who reads, as the keeper knows them (`channel_readers`); made when first asked. */
  readers?: { pubkey: string; since: number }[];
}

/** The readers of a demo channel: two of the demo's people and up to 140 more, one every five hours back. */
function demoReaders(c: DemoChannel): { pubkey: string; since: number }[] {
  const out = [{ pubkey: VERA, since: T0 - 3 * H }, { pubkey: NIKITA, since: T0 - 2 * D }];
  const more = Math.min(140, Math.max(0, c.view.members - out.length));
  for (let i = 0; i < more; i++) out.push({ pubkey: hex(`d${i.toString(16).padStart(3, '0')}`), since: T0 - (i + 1) * 5 * H });
  return out;
}

let seq = 0;
const id = () => `d${(seq++).toString(16).padStart(63, '0')}`;

function item(cid: string, author: string, at: number, text: string | undefined, extra: Partial<FeedItemViewLv> = {}): FeedItemViewLv {
  return {
    id: id(), community_id: cid, post_id: '', author, created_at: at, text, review: 'ok', hidden: false, deleted: false,
    pinned: false, reactions: [], comments: 0, pending: false, ...extra,
  };
}

function channel(cid: string, name: string, about: string, owner: string, extra: Partial<ChannelView>): ChannelView {
  return {
    id: cid, chat_id: `channel:${cid}`, name, about, owner, join: 'open', membership: 'joined', members: 0,
    relays: ['wss://node-1.veydan.net'], storage: ['https://node-1.veydan.net/media'], link_epoch: 1, kept_here: false,
    needs_update: false, unread: 0, pinned: false, muted: false, archived: false, created_at: T0 - 40 * D, updated_at: T0 - H, last_at: 0, ...extra,
  };
}

function buildDemo(): Record<string, DemoChannel> {
  const c1 = hex('c1');
  const posts1: FeedItemView[] = [
    item(c1, ME, T0 - 6 * D, 'Добро пожаловать в канал Veydan! Здесь — новости приложения, планы и ответы на вопросы.\n\nКомментарии открыты, пишите.', { pinned: true, reactions: [{ emoji: '👍', count: 214, mine: false }, { emoji: '🎉', count: 58, mine: false }], comments: 31 }),
    item(c1, ALICE, T0 - 3 * D, 'Новый дизайн «Тихое стекло» доехал до всех экранов. Расскажите, что нравится, а что нет.', { media_json: media('glass.jpg', 'image', 482_000, undefined, [960, 640]), reactions: [{ emoji: '😍', count: 96, mine: true }, { emoji: '👍', count: 40, mine: false }], comments: 12 }),
    item(c1, MARINA, T0 - 2 * D, 'Чек-лист тестирования звонков — в файле. Если найдёте ошибку, напишите в комментариях.', { media_json: media('calls-checklist.pdf', 'file', 1_240_000), comments: 4 }),
    item(c1, ME, T0 - 20 * H, 'Каналы появятся в Chat и в Space. Ключи раздаёт хранитель — устройство владельца канала. Пока оно не в сети, новые читатели ждут ключ, а остальные читают как обычно.', { reactions: [{ emoji: '🔥', count: 120, mine: true }], comments: 7 }),
    item(c1, ALICE, T0 - 8 * H, '# Что нового в 5.2\n\n**Каналы** — посты, комментарии и реакции.\n\n- закреп и премодерация\n- файлы до 1 ГБ\n- ссылки на каналы в чатах', { fmt: 'md', rich: demoRich('# Что нового в 5.2\n\n**Каналы** — посты, комментарии и реакции.\n\n- закреп и премодерация\n- файлы до 1 ГБ\n- ссылки на каналы в чатах'), reactions: [{ emoji: '🎉', count: 77, mine: false }] }),
    item(c1, BORIS, T0 - 5 * H, 'Сегодня в 22:00 по Москве — короткие работы на реле node-1. Ничего делать не нужно.', { reactions: [{ emoji: '👌', count: 33, mine: false }] }),
    item(c1, MARINA, T0 - 40 * 60, 'Черновик поста о бэкапах — ждёт проверки.', { review: 'pending' }),
  ];
  const first = posts1[0].id;
  const thread: FeedItemView[] = [
    item(c1, ALICE, T0 - 6 * D + H, 'Наконец-то! Ждали.', { post_id: first, reactions: [{ emoji: '❤️', count: 9, mine: false }] }),
    item(c1, NIKITA, T0 - 6 * D + 2 * H, 'А будут ли каналы с платной подпиской?', { post_id: first }),
    item(c1, VERA, T0 - 5 * D, 'Где можно почитать про хранителя подробнее?', { post_id: first }),
  ];
  thread.push(item(c1, ME, T0 - 5 * D + H, 'Подписки — на следующем этапе, сначала каналы.', { post_id: first, parent_id: thread[1].id }));
  thread.push(item(c1, BORIS, T0 - 4 * D, 'В справке приложения, раздел «Каналы».', { post_id: first, parent_id: thread[2].id }));
  const c2 = hex('c2');
  const c3 = hex('c3');
  const c4 = hex('c4');
  const c5 = hex('c5');
  const c6 = hex('c6');
  const c9 = hex('c9');
  const ca = hex('ca');
  const base = (posts: FeedItemView[]): Pick<DemoChannel, 'posts' | 'comments_of' | 'queue' | 'log' | 'requests' | 'banned'> => ({
    posts, comments_of: {}, queue: [], log: [], requests: [], banned: [],
  });
  return {
    [c1]: {
      view: channel(c1, 'Veydan: новости', 'Новости приложения Veydan, планы и ответы на вопросы.', ME, {
        picture: picture('#6d5dfc', '#2bb3c0', 'V'), join: 'request', my_role: 'owner', members: 1284, kept_here: true, unread: 0, pinned: true,
        last: { id: posts1[5].id, author: BORIS, text: posts1[5].text, has_media: false, at: posts1[5].created_at },
      }),
      history: 'open', post_mode: 'authors', comments: 'on', premod: 'newcomers', pinned: [first],
      staff: [
        { pubkey: ME, role: 'owner', muted: false, trusted: false, since: T0 - 40 * D },
        { pubkey: ALICE, role: 'admin', muted: false, trusted: false, since: T0 - 30 * D },
        { pubkey: BORIS, role: 'moderator', muted: false, trusted: false, since: T0 - 20 * D },
        { pubkey: MARINA, role: 'author', muted: false, trusted: true, since: T0 - 10 * D },
        { pubkey: NIKITA, role: 'member', muted: true, muted_until: T0 + 20 * H, trusted: false, since: T0 - 2 * D },
      ],
      has_key: true, waiting: 0, link: `veydan://channel/${c1}?r=node-1.veydan.net&e=1#s=demo-secret`,
      ...base(posts1),
      comments_of: { [first]: thread },
      queue: [
        { message_id: id(), author: VERA, text: 'Посмотрите мой канал про фото, там классно!', at: T0 - 2 * H, version: 'v1', more_from_author: 0 },
        { message_id: id(), author: VERA, text: 'И ещё один мой канал — про походы.', at: T0 - 90 * 60, version: 'v1', more_from_author: 0 },
        { message_id: id(), author: VERA, text: 'Подпишитесь, пожалуйста, это важно!!!', at: T0 - 70 * 60, version: 'v1', more_from_author: 0 },
        { message_id: id(), author: NIKITA, text: 'А когда выйдет версия для iPhone?', at: T0 - 50 * 60, version: 'v1', more_from_author: 0 },
      ],
      log: [
        { at: T0 - 3 * H, by: BORIS, action: 'mute', target: NIKITA, detail: JSON.stringify({ until: T0 + 20 * H }) },
        { at: T0 - D, by: ALICE, action: 'hide', target: thread[2].id, detail: JSON.stringify({ reason: 'spam' }), excerpt: thread[2].text },
        { at: T0 - 2 * D, by: ME, action: 'settings', target: '', detail: JSON.stringify({ premod: 'newcomers' }) },
        { at: T0 - 3 * D, by: ME, action: 'pin', target: first, detail: '', excerpt: 'Добро пожаловать в канал Veydan! Здесь — новости приложения, планы и ответы на вопросы.' },
      ],
      requests: [],
      banned: [hex('5e')],
    },
    [c2]: {
      view: channel(c2, 'Фотоклуб «Свет»', 'Фото недели, разборы и задания. Заявки рассматриваем раз в день.', ALICE, {
        picture: picture('#f2994a', '#eb5757', '◐'), join: 'request', my_role: undefined, members: 342, unread: 4, keeper_last_seen: T0 - 5 * H,
        last: { id: 'x', author: ALICE, text: 'Задание недели: отражения', has_media: true, at: T0 - 2 * H },
      }),
      history: 'from_join', post_mode: 'authors', comments: 'on', premod: 'off', pinned: [],
      staff: [{ pubkey: ALICE, role: 'owner', muted: false, trusted: false, since: T0 - 60 * D }],
      has_key: true, waiting: 3,
      ...base([
        item(c2, ALICE, T0 - 2 * D, 'Итоги прошлой недели: победила работа «Туман над рекой».', { media_json: media('fog.jpg', 'image', 640_000, 'Туман над рекой', [960, 640]), reactions: [{ emoji: '👏', count: 51, mine: false }], comments: 18 }),
        item(c2, ALICE, T0 - 2 * H, 'Задание недели: отражения. Присылайте в комментарии до воскресенья.', { comments: 2 }),
      ]),
    },
    [c3]: {
      view: channel(c3, 'Рецепты без спешки', 'Простые рецепты на каждый день.', BORIS, {
        picture: picture('#27ae60', '#a8e063', '🍲'), membership: 'joining', members: 97, created_at: T0 - 2 * 60, updated_at: T0 - 2 * 60, keeper_last_seen: T0 - 50 * 60,
      }),
      history: 'open', post_mode: 'authors', comments: 'off', premod: 'off', pinned: [], staff: [], has_key: false, waiting: 0, ...base([]),
    },
    // Joined an hour ago; the keeper last handed out a key three days ago.
    [c9]: {
      view: channel(c9, 'Клуб садоводов', 'Рассада, сорта и урожай.', VERA, { membership: 'joining', members: 214, created_at: T0 - H, updated_at: T0 - H, keeper_last_seen: T0 - 3 * D }),
      history: 'open', post_mode: 'authors', comments: 'on', premod: 'off', pinned: [], staff: [], has_key: false, waiting: 0, ...base([]),
    },
    [c4]: {
      view: channel(c4, 'Закрытый клуб тестировщиков', 'Сборки до релиза и разбор ошибок.', MARINA, { join: 'request', membership: 'requested', members: 58 }),
      history: 'from_join', post_mode: 'authors', comments: 'on', premod: 'all', pinned: [], staff: [], has_key: false, waiting: 0, ...base([]),
    },
    [c5]: {
      view: channel(c5, 'Старый канал команды', 'Архив объявлений прошлого года.', ALICE, { membership: 'out', out_reason: 'removed', members: 12, archived: true }),
      history: 'open', post_mode: 'authors', comments: 'off', premod: 'off', pinned: [], staff: [], has_key: true, waiting: 0,
      ...base([item(c5, ALICE, T0 - 200 * D, 'Итоги года: спасибо всем, кто был с нами.', { reactions: [{ emoji: '❤️', count: 8, mine: false }] })]),
    },
    [ca]: {
      view: channel(ca, 'Курс фотографии', 'Двенадцать уроков для начинающих.', MARINA, { membership: 'disbanded', out_reason: 'disbanded', members: 0, updated_at: T0 - 4 * D }),
      history: 'open', post_mode: 'authors', comments: 'off', premod: 'off', pinned: [], staff: [], has_key: true, waiting: 0, ...base([]),
    },
    [c6]: {
      view: channel(c6, 'Анонсы релизов', 'Коротко о каждой новой версии.', BORIS, {
        picture: picture('#2d9cdb', '#56ccf2', 'R'), my_role: 'author', members: 2210, needs_update: true, muted: true, unread: 2, keeper_last_seen: T0 - 3 * 60,
        last: { id: 'y', author: BORIS, text: 'Вышла 5.2: каналы и новые реакции', has_media: false, at: T0 - D },
      }),
      history: 'open', post_mode: 'authors', comments: 'on', premod: 'off', pinned: [],
      staff: [{ pubkey: BORIS, role: 'owner', muted: false, trusted: false, since: T0 - 90 * D }, { pubkey: ME, role: 'author', muted: false, trusted: false, since: T0 - 9 * D }],
      has_key: true, waiting: 0,
      ...base([item(c6, BORIS, T0 - D, 'Вышла 5.2: каналы и новые реакции.', { reactions: [{ emoji: '🚀', count: 140, mine: false }], comments: 0 })]),
    },
  };
}

/**
 * Levels and sections (communities-wire §14) in three channels: mine, with
 * two sections, three levels and requests for them; a reader's, with a
 * section their level opens and one closed to them; an author's, with a
 * section to post to.
 */
function demoLevels() {
  const all = data();
  const can = (comment: boolean, discuss: boolean, post: boolean) => ({ comment, discuss, post });
  const scope = (scope_id: string, title: string, extra: Partial<ScopeView> = {}): ScopeView => ({ scope_id, title, scheme: 'sd-v1', history: 'open', mine: false, ...extra });

  const c1 = all[hex('c1')];
  const PREMIUM = 'a1'.repeat(16);
  const BACKSTAGE = 'b2'.repeat(16);
  const MODS = 'f6'.repeat(16);
  // The look of the tiles (communities-wire §14.7): icons, colours, a photo, a hidden section of the team.
  c1.scopes = [
    scope('everyone', '', { icon: 'speakerphone', color: 1, note: 'Новости и анонсы', order: 0, unread: 2 }),
    scope(PREMIUM, 'Премиум', { icon: 'star', color: 5, note: 'Ранний доступ и планы', order: 1, unread: 3 }),
    scope(BACKSTAGE, 'Закулисье', { scheme: 'flat-v1', history: 'from_join', icon: 'camera', color: 3, photo: 'demo-backstage', note: 'Как мы работаем', order: 2 }),
    scope(MODS, 'Модераторская', { scheme: 'flat-v1', icon: 'shield', color: 7, note: 'Только команда', order: 3, hidden: true, unread: 1 }),
  ];
  c1.levels = [
    { id: 0, name: 'Читатели', scopes: ['everyone'], can: can(true, false, false), members: 1284 },
    { id: 1, name: 'Поддержка', scopes: [PREMIUM], can: can(true, true, false), members: 86 },
    { id: 2, name: 'Меценаты', scopes: [PREMIUM, BACKSTAGE], can: can(true, true, true), members: 12 },
  ];
  c1.my_levels = [0];
  c1.staff.forEach((s) => { if (s.pubkey === MARINA) s.levels = [0, 1]; if (s.pubkey === NIKITA) s.levels = [0, 2]; });
  c1.posts.push(
    item(c1.view.id, ME, T0 - 30 * H, 'Для поддержки: черновик плана на зиму — каналы, сообщества и обсуждения.', { scope_id: PREMIUM, reactions: [{ emoji: '❤️', count: 31, mine: false }], comments: 3 }),
    item(c1.view.id, ALICE, T0 - 3 * H, 'Ранний доступ: сборка 5.3 для тех, кто нас поддерживает. Ссылка — в комментариях.', { scope_id: PREMIUM, comments: 1 }),
    item(c1.view.id, BORIS, T0 - 2 * H, 'Закулисье: на этой неделе разбираем жалобы на спам в каналах.', { scope_id: BACKSTAGE }),
    item(c1.view.id, ALICE, T0 - 90 * 60, 'Команде: жалобы за неделю разобраны, двое забанены за рекламу.', { scope_id: MODS }),
  );
  c1.level_requests = [
    { requester: VERA, level: 1, note: 'Поддерживаю проект с первой версии, хочу читать планы раньше всех.', created_at: T0 - 3 * H },
    { requester: hex('6f'), level: 2, note: '', created_at: T0 - 20 * H },
  ];

  const c2 = all[hex('c2')];
  const REVIEWS = 'c3'.repeat(16);
  const CLASSES = 'd4'.repeat(16);
  const FRIENDS = 'a7'.repeat(16);
  const SECRET = 'b8'.repeat(16);
  // A reader's tiles: a section of theirs, one closed to them, a hidden one their level opens, and a hidden one it does not (never shown).
  c2.scopes = [
    scope('everyone', ''),
    scope(REVIEWS, 'Разборы работ', { icon: 'photo', color: 3, note: 'Разбираем ваши снимки', unread: 1 }),
    scope(CLASSES, 'Мастер-классы', { icon: 'book', color: 8, note: 'Уроки по субботам' }),
    scope(FRIENDS, 'Клуб друзей', { icon: 'heart', color: 7, note: 'Встречи для своих', hidden: true }),
    scope(SECRET, 'Жюри', { icon: 'shield', color: 9, hidden: true }),
  ];
  c2.levels = [
    { id: 0, name: 'Участники клуба', scopes: ['everyone'], can: can(true, false, false), members: 342 },
    { id: 1, name: 'Друзья клуба', scopes: [REVIEWS, FRIENDS], can: can(true, false, false), members: 120 },
    { id: 2, name: 'Ученики', scopes: [REVIEWS, CLASSES], can: can(true, true, false), members: 37 },
    { id: 3, name: 'Жюри', scopes: [SECRET], can: can(true, true, true), members: 5 },
  ];
  c2.my_levels = [0, 1];
  c2.posts.push(
    item(c2.view.id, ALICE, T0 - 26 * H, 'Разбор: почему «Туман над рекой» победил — свет, линия горизонта и тишина в кадре.', { scope_id: REVIEWS, reactions: [{ emoji: '👏', count: 22, mine: false }], comments: 5 }),
    item(c2.view.id, ALICE, T0 - 6 * H, 'Мастер-класс по ночной съёмке — в субботу.', { scope_id: CLASSES }),
    item(c2.view.id, ALICE, T0 - 4 * H, 'Друзьям клуба: в пятницу собираемся в студии, приносите плёнку.', { scope_id: FRIENDS }),
  );

  const c6 = all[hex('c6')];
  const BETA = 'e5'.repeat(16);
  c6.scopes = [scope('everyone', ''), scope(BETA, 'Бета-сборки')];
  c6.levels = [
    { id: 0, name: 'Подписчики', scopes: ['everyone'], can: can(true, false, false), members: 2210 },
    { id: 1, name: 'Тестировщики', scopes: [BETA], can: can(true, true, false), members: 64 },
  ];
  c6.my_levels = [0, 1];
  c6.posts.push(item(c6.view.id, BORIS, T0 - 10 * H, 'Бета 5.3.0-rc.1: новые разделы каналов. Пишите, что сломалось.', { scope_id: BETA, comments: 0 }));
}

let demo: Record<string, DemoChannel> | null = null;
const data = () => (demo ??= buildDemo());
const ch = (a?: Record<string, unknown>): DemoChannel => {
  const c = data()[String(a?.communityId ?? '')];
  if (!c) throw { code: 'other', message: 'channel_unknown: unknown channel' };
  return c;
};

let invitesIn: InviteView[] = [
  {
    invite_id: 'inv1', community_id: hex('c7'), direction: 'in', peer: ALICE, status: 'sent', name: 'Дизайн-ревью',
    about: 'Разбираем макеты по пятницам.', created_at: T0 - 3 * H, expires_at: T0 + 6 * D, verified: true,
  },
];

function detail(c: DemoChannel): ChannelDetail {
  // The last post I may read: one of a section closed to me is not mine to see.
  const readable = new Set(c.scopes ? scopesFor(c).filter((s) => s.mine).map((s) => s.scope_id).concat('everyone') : ['everyone']);
  const lastPost = c.posts.filter((p) => !p.deleted && readable.has((p as FeedItemViewLv).scope_id || 'everyone')).sort((x, y) => x.created_at - y.created_at).at(-1);
  c.view.last = lastPost ? { id: lastPost.id, author: lastPost.author, text: lastPost.text, has_media: !!lastPost.media_json, media_kind: isRecording(mediaOf(lastPost)) ? mediaOf(lastPost)?.kind : undefined, at: lastPost.created_at } : c.view.last;
  // What the list is ordered by, as the core says it: the last post, or when the channel came here.
  c.view.last_at = c.view.last?.at ?? c.view.created_at;
  return {
    channel: { ...c.view }, history: c.history, post_mode: c.post_mode, comments: c.comments, premod: c.premod, schedule: c.schedule,
    pinned: [...c.pinned], staff: c.staff.map((s) => ({ ...s, levels: s.levels ? [...s.levels] : undefined })), disbanded: c.view.membership === 'disbanded', has_key: c.has_key,
    waiting: c.waiting, keeper: c.view.owner, link: c.view.membership === 'joined' ? c.link : undefined,
    // Stage 2 (communities-wire §14): what the core will add to ChannelDetail.
    ...(c.scopes ? { scopes: scopesFor(c), levels: (c.levels ?? []).map((l) => ({ ...l, scopes: [...l.scopes], can: { ...l.can } })), my_levels: [...(c.my_levels ?? [0])] } : {}),
    ...(c.transfer ? { transfer: { ...c.transfer } } : {}),
  } as ChannelDetail;
}

/** The sections as I see them: the team is in each, a reader in those their levels give. */
function scopesFor(c: DemoChannel): ScopeView[] {
  const staff = ['moderator', 'admin', 'owner'].includes(c.view.my_role ?? '');
  const given = new Set(['everyone', ...(c.levels ?? []).filter((l) => l.id === 0 || (c.my_levels ?? []).includes(l.id)).flatMap((l) => l.scopes)]);
  // Where I may post (2026-10-23): an author and above where I belong; a reader where a posting level gives it.
  const author = staff || c.view.my_role === 'author';
  const posting = new Set((c.levels ?? []).filter((l) => (l.id === 0 || (c.my_levels ?? []).includes(l.id)) && l.can.post).flatMap((l) => (l.id === 0 ? ['everyone', ...l.scopes] : l.scopes)));
  const shown = (c.scopes ?? []).map((s) => {
    const mine = staff || given.has(s.scope_id);
    return { ...s, mine, post: author ? mine : posting.has(s.scope_id), hidden: !!s.hidden, unread: s.unread ?? 0 };
  }).filter((s) => s.mine || !s.hidden);
  return shown.map((s, i) => ({ s, i })).sort((a, b) => (a.s.order ?? 99) - (b.s.order ?? 99) || a.i - b.i).map((x) => x.s);
}

/** A refusal of the module, as the core words it. */
const refuse = (code: string, text: string) => { throw { code: 'other', message: `channel_${code}: ${text}` }; };

const roleOf = (c: DemoChannel) => c.view.my_role;
const updated = (c: DemoChannel) => emit('channel.updated', { community_id: c.view.id });

/** A page before the cursor in the order (date, id), as the module pages. */
function page(items: FeedItemView[], next: unknown, limit: unknown): FeedPageView {
  const c = next as FeedCursorView | null | undefined;
  const n = typeof limit === 'number' ? limit : 50;
  const key = (x: FeedItemView) => [x.created_at, x.id] as const;
  const before = (x: FeedItemView) => !c || x.created_at < c.created_at || (x.created_at === c.created_at && x.id < c.id);
  const older = items.filter(before).sort((x, y) => key(x)[0] - key(y)[0] || (x.id < y.id ? -1 : 1));
  const out = older.slice(-n);
  const first = older.length > n ? out[0] : undefined;
  return { items: out, next_before: first?.created_at, next: first ? { created_at: first.created_at, id: first.id } : undefined, asked: false };
}

function findItem(c: DemoChannel, target: string): FeedItemView | undefined {
  return c.posts.find((p) => p.id === target) ?? Object.values(c.comments_of).flat().find((x) => x.id === target);
}

/** The demo's files on their way, by upload id. */
const demoUploads = new Map<string, UploadView>();

/** A demo file going up in steps; once all of it is there it becomes a post (or a comment). */
function demoSend(c: DemoChannel, up: UploadView) {
  const timer = setInterval(() => {
    if (!demoUploads.has(up.id)) { clearInterval(timer); return; }
    up.done = Math.min(up.total, up.done + 200_000);
    emit('channel.media', { community_id: c.view.id, id: up.id, status: up.done < up.total ? 'uploading' : 'stored', done: up.done, total: up.total });
    if (up.done < up.total) return;
    clearInterval(timer);
    demoUploads.delete(up.id);
    const md = (up as DemoUpload).fmt === 'md' && up.caption ? { fmt: 'md', rich: demoRich(up.caption) } : {};
    const rec = (up as DemoUpload).rec;
    const p = item(c.view.id, ME, now(), up.caption, {
      ...md,
      media_json: rec ? recMedia(rec.kind, up.total, rec.ms, rec.waveform) : media(up.name, /\.(png|jpe?g|webp)$/i.test(up.name) ? 'image' : 'file', up.total, up.caption, [960, 640], (up as DemoUpload).batch),
    });
    if (rec) demoRecordings.set(p.id, rec.data);
    if (up.post_id) { p.post_id = up.post_id; (c.comments_of[up.post_id] ??= []).push(p); } else c.posts.push(p);
    emit('channel.message', { community_id: c.view.id, id: p.id });
  }, 500);
}

if (!isTauri) {
  const mocks: Record<string, (a?: Record<string, unknown>) => unknown> = {
    channels_list: () => Object.values(data()).map((c) => detail(c).channel).sort((x, y) => y.last_at - x.last_at || x.id.localeCompare(y.id)),
    channel_get: (a) => detail(ch(a)),
    channel_storage_plan: (): StoragePlanView => ({ free: true, retention_days: 14 }),
    channel_create: (a) => {
      const input = a?.input as NewChannelInput;
      const cid = id().replace(/^d/, 'e');
      const c: DemoChannel = {
        view: channel(cid, input.name, input.about ?? '', ME, { join: input.join, my_role: 'owner', members: 1, kept_here: true, relays: input.relays, storage: input.storage, created_at: now(), updated_at: now() }),
        history: input.history ?? 'open', post_mode: 'authors', comments: 'on', premod: 'off', pinned: [],
        staff: [{ pubkey: ME, role: 'owner', muted: false, trusted: false, since: now() }], has_key: true, waiting: 0,
        link: `veydan://channel/${cid}?r=node-1.veydan.net&e=1#s=new`, posts: [], comments_of: {}, queue: [], log: [], requests: [], banned: [],
      };
      data()[cid] = c;
      return detail(c);
    },
    channel_act: (a) => {
      const c = ch(a);
      const act = a?.action as ChannelAction;
      switch (act.op) {
        case 'settings': {
          const { op: _op, ...rest } = act;
          if (rest.name) c.view.name = rest.name;
          if (rest.about !== undefined) c.view.about = rest.about;
          if (rest.join) c.view.join = rest.join;
          if (rest.history) c.history = rest.history;
          if (rest.post_mode) c.post_mode = rest.post_mode;
          if (rest.comments) c.comments = rest.comments;
          if (rest.premod) c.premod = rest.premod;
          if (rest.schedule) c.schedule = rest.schedule.days.length ? rest.schedule : undefined;
          break;
        }
        case 'set_role': {
          const s = c.staff.find((x) => x.pubkey === act.who);
          if (s) s.role = act.role;
          else c.staff.push({ pubkey: act.who, role: act.role, muted: false, trusted: false, since: now() });
          break;
        }
        case 'ban': c.banned.push(act.who); c.staff = c.staff.filter((x) => x.pubkey !== act.who); break;
        case 'unban': c.banned = c.banned.filter((x) => x !== act.who); break;
        case 'remove': c.staff = c.staff.filter((x) => x.pubkey !== act.who); c.readers = c.readers?.filter((r) => r.pubkey !== act.who); break;
        case 'set_muted': {
          const s = c.staff.find((x) => x.pubkey === act.who);
          if (s) { s.muted = act.muted; s.muted_until = act.until; }
          else c.staff.push({ pubkey: act.who, role: 'member', muted: act.muted, muted_until: act.until, trusted: false, since: now() });
          break;
        }
        case 'set_trusted': { const s = c.staff.find((x) => x.pubkey === act.who); if (s) s.trusted = act.trusted; break; }
        case 'pin': c.pinned = act.pinned ? [...c.pinned, act.post_id] : c.pinned.filter((p) => p !== act.post_id); break;
        case 'leave': c.view.membership = 'out'; c.view.my_role = undefined; break;
        case 'disband': c.view.membership = 'disbanded'; break;
        default: break;
      }
      updated(c);
      return detail(c);
    },
    channel_open_link: (a) => {
      const link = String(a?.link ?? '').trim();
      if (!/^veydan:\/\/channel\//.test(link)) throw { code: 'other', message: 'channel_bad_link: not a link of a channel' };
      const cid = hex('c8');
      const request = /join=request/.test(link);
      const c: DemoChannel = {
        view: channel(cid, request ? 'Клуб по заявкам' : 'Канал по ссылке', 'Демо: канал, открытый по ссылке.', BORIS, { join: request ? 'request' : 'open', membership: request ? 'requested' : 'joining', members: 40 }),
        history: 'open', post_mode: 'authors', comments: 'on', premod: 'off', pinned: [], staff: [], has_key: false, waiting: 0,
        posts: [], comments_of: {}, queue: [], log: [], requests: [], banned: [],
      };
      data()[cid] = c;
      return detail(c);
    },
    channel_preview_link: (a) => {
      const link = String(a?.link ?? '').trim();
      const m = /^veydan:\/\/channel\/([0-9a-f]{64})/.exec(link);
      if (!m) throw { code: 'other', message: 'channel_bad_link: not a link of a channel' };
      // The demo's way to see the state of a relay that does not answer.
      if (/unreachable/.test(link)) throw { code: 'other', message: 'channel_preview_unreachable: the relay of the link did not answer with the card' };
      const here = data()[m[1]];
      const request = /join=request/.test(link);
      const invite = /join=invite/.test(link);
      return {
        community_id: m[1], name: here?.view.name ?? (request ? 'Клуб по заявкам' : invite ? 'Закрытый клуб' : 'Канал по ссылке'),
        about: here?.view.about ?? 'Демо: карточка канала, полученная по ссылке до входа.', picture: here?.view.picture,
        members: here?.view.members ?? 40, join: here?.view.join ?? (request ? 'request' : invite ? 'invite' : 'open'), owner: here?.view.owner ?? BORIS,
        relay: 'wss://node-1.veydan.net', has_secret: /[?&#]s=/.test(link), invited: /[?&]i=/.test(link), membership: here?.view.membership,
      } satisfies ChannelPreview;
    },
    channel_invite: (a) => ({ invite_id: id(), community_id: ch(a).view.id, direction: 'out', peer: String(a?.who), status: 'sent', name: ch(a).view.name, about: '', created_at: now(), expires_at: now() + 7 * D, verified: true }),
    channel_invites: (a) => (a?.direction === 'in' ? invitesIn : []),
    channel_answer_invite: (a) => { invitesIn = invitesIn.filter((i) => i.invite_id !== a?.inviteId); },
    channel_answer_request: (a) => { const c = ch(a); c.requests = c.requests.filter((r) => r.requester !== a?.requester); return detail(c); },
    channel_avatar_set: (a) => { const c = ch(a); c.view.picture = picture('#9b51e0', '#f2c94c', '★'); return detail(c); },
    channel_avatar_remove: (a) => { const c = ch(a); c.view.picture = undefined; return detail(c); },
    channel_rotate_link: (a) => { const c = ch(a); c.view.link_epoch += 1; c.link = `veydan://channel/${c.view.id}?r=node-1.veydan.net&e=${c.view.link_epoch}#s=rotated`; return detail(c); },
    channel_link_qr: () => '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 21 21" shape-rendering="crispEdges"><rect width="21" height="21" fill="#fff"/><path d="M0 0h7v7H0zM14 0h7v7h-7zM0 14h7v7H0zM9 2h2v2H9zM9 9h3v3H9zM14 10h2v2h-2zM10 15h2v2h-2zM16 16h3v3h-3z" fill="#000"/><path d="M2 2h3v3H2zM16 2h3v3h-3zM2 16h3v3H2z" fill="#fff"/></svg>',
    channel_forget: (a) => { delete data()[String(a?.communityId)]; },
    channel_history: (a) => {
      const c = ch(a);
      if (!c.has_key) return { items: [], asked: true };
      // The sections I am in, or the one asked for; a post of a section I lack is never mine to read.
      const mine = new Set(scopesFor(c).filter((s) => s.mine).map((s) => s.scope_id).concat('everyone'));
      const sc = typeof a?.scopeId === 'string' ? a.scopeId : null;
      const of = (p: FeedItemViewLv) => p.scope_id || 'everyone';
      return page(c.posts.filter((p) => mine.has(of(p)) && (!sc || of(p) === sc)), a?.next, a?.limit);
    },
    channel_thread: (a) => page(ch(a).comments_of[String(a?.postId)] ?? [], a?.next, a?.limit),
    channel_post: (a) => {
      const c = ch(a);
      const text = String(a?.text ?? '');
      const md = a?.fmt === 'md' ? { fmt: 'md', rich: demoRich(text) } : {};
      const sc = typeof a?.scopeId === 'string' && a.scopeId !== 'everyone' ? a.scopeId : undefined;
      if (sc && !scopesFor(c).some((s) => s.scope_id === sc && s.mine)) refuse('not_member_of_scope', 'not a member of the scope');
      const p = item(c.view.id, ME, now(), text, { ...md, scope_id: sc, review: c.premod === 'all' && roleOf(c) === 'member' ? 'pending' : 'ok' });
      c.posts.push(p);
      emit('channel.message', { community_id: c.view.id, id: p.id });
      return p.id;
    },
    channel_comment: (a) => {
      const c = ch(a);
      const pid = String(a?.postId);
      const text = String(a?.text ?? '');
      const md = a?.fmt === 'md' ? { fmt: 'md', rich: demoRich(text) } : {};
      const x = item(c.view.id, ME, now(), text, { ...md, post_id: pid, parent_id: (a?.parent as string | null) ?? undefined });
      (c.comments_of[pid] ??= []).push(x);
      const post = c.posts.find((p) => p.id === pid);
      if (post) post.comments += 1;
      emit('channel.message', { community_id: c.view.id, id: x.id });
      return x.id;
    },
    channel_react: (a) => {
      const c = ch(a);
      const x = findItem(c, String(a?.target));
      if (x) {
        const e = String(a?.emoji);
        const r = x.reactions.find((y) => y.emoji === e);
        if (a?.set) { if (r && !r.mine) { r.count += 1; r.mine = true; } else if (!r) x.reactions.push({ emoji: e, count: 1, mine: true }); }
        else if (r?.mine) { r.count -= 1; r.mine = false; x.reactions = x.reactions.filter((y) => y.count > 0); }
        emit('channel.message', { community_id: c.view.id, id: x.id });
      }
      return id();
    },
    channel_md_preview: (a) => demoRich(String(a?.text ?? '')),
    channel_edit: (a) => {
      const c = ch(a); const x = findItem(c, String(a?.target));
      if (x) { x.text = String(a?.text ?? ''); x.fmt = a?.fmt === 'md' ? 'md' : undefined; x.rich = x.fmt ? demoRich(x.text) : undefined; x.edited_at = now(); } emit('channel.message', { community_id: c.view.id, id: a?.target }); return id(); },
    channel_delete: (a) => { const c = ch(a); const x = findItem(c, String(a?.target)); if (x) { x.deleted = true; x.text = undefined; x.media_json = undefined; } emit('channel.message', { community_id: c.view.id, id: a?.target }); return id(); },
    channel_pin: (a) => {
      const c = ch(a);
      const pid = String(a?.postId);
      c.pinned = a?.pinned ? [pid, ...c.pinned.filter((p) => p !== pid)] : c.pinned.filter((p) => p !== pid);
      c.posts.forEach((p) => (p.pinned = c.pinned.includes(p.id)));
      updated(c);
    },
    channel_read: (a) => { ch(a).view.unread = 0; },
    channel_media_send: (a) => {
      const c = ch(a);
      const name = String(a?.src ?? 'file').split('/').pop() ?? 'file';
      const up: DemoUpload = {
        id: id(), community_id: c.view.id, name, post_id: String(a?.postId ?? ''), caption: (a?.caption as string) ?? undefined, status: 'uploading', done: 0, total: 2_400_000, created_at: now(),
        batch: typeof a?.batch === 'string' ? a.batch : undefined, fmt: a?.fmt === 'md' ? 'md' : undefined,
      };
      demoUploads.set(up.id, up);
      demoSend(c, up);
      return up;
    },
    // A recording of the page (the demo's camera and microphone): it goes as a file does, then plays from what was recorded.
    channel_media_record: (a) => {
      const c = ch(a);
      const r = (a?.recording ?? {}) as { kind: 'voice' | 'circle'; mime: string; duration_ms: number; waveform: number[] | null; data_base64: string };
      const size = Math.max(1, Math.floor((r.data_base64.length * 3) / 4));
      const up: DemoUpload = {
        id: id(), community_id: c.view.id, name: r.kind === 'voice' ? 'voice.weba' : 'circle.webm', kind: r.kind, post_id: String(a?.postId ?? ''), status: 'uploading', done: 0,
        total: Math.max(size, 600_000), created_at: now(),
        rec: { kind: r.kind, ms: r.duration_ms, waveform: r.waveform ?? undefined, data: `data:${r.mime.split(';')[0]};base64,${r.data_base64}` },
      };
      demoUploads.set(up.id, up);
      demoSend(c, up);
      return { ...up, rec: undefined };
    },
    channel_media_retry: (a) => {
      const up = demoUploads.get(String(a?.uploadId));
      if (!up || up.status !== 'failed') throw { code: 'other', message: 'channel_invalid: the upload has not failed' };
      Object.assign(up, { status: 'uploading', error: undefined, done: 0 });
      demoSend(ch({ communityId: up.community_id }), up);
      return { ...up };
    },
    channel_media_fetch: (a) => {
      const c = ch(a);
      const x = findItem(c, String(a?.messageId));
      const m = x ? mediaOf(x) : null;
      return '/home/dev/.local/share/veydan/communities/media/' + (m?.name ?? 'file');
    },
    channel_media_url: (a) => {
      const x = findItem(ch(a), String(a?.messageId));
      const m = x ? mediaOf(x) : null;
      // Pictures of one album told apart by their names.
      const tints = [['#5b6cff', '#1fc8a9'], ['#f2994a', '#eb5757'], ['#9b51e0', '#56ccf2'], ['#27ae60', '#f2c94c']];
      const [a1, b1] = tints[[...(m?.name ?? '')].reduce((s, ch) => s + ch.charCodeAt(0), 0) % tints.length];
      // The demo's picture carries no words of its own when it is part of an album: the album's caption is the post's text, above it.
      if (m?.kind === 'voice') return demoRecordings.get(String(a?.messageId)) ?? demoTone(m.duration_ms ?? 3000);
      if (m?.kind === 'circle') return demoRecordings.get(String(a?.messageId)) ?? demoCircle();
      return m?.kind === 'image' ? picture(a1, b1, m.batch ? '' : (m.caption ?? '')) : null;
    },
    channel_cover: (a) => { const p = ch(a).view.picture; return p ? (p.startsWith('data:') ? p : picture('#5b6cff', '#1fc8a9', '')) : null; },
    channel_picked_preview: (a) => {
      // The demo's pictures, told apart by their names; anything else shows its icon.
      const src = String(a?.src ?? '');
      if (!/\.(png|jpe?g|webp|gif|avif|bmp)$/i.test(src)) return null;
      const tints = [['#5b6cff', '#1fc8a9'], ['#f2994a', '#eb5757'], ['#9b51e0', '#56ccf2'], ['#27ae60', '#f2c94c']];
      const [a1, b1] = tints[[...src].reduce((s, ch) => s + ch.charCodeAt(0), 0) % tints.length];
      return picture(a1, b1, '');
    },
    channel_media_cancel: (a) => { demoUploads.delete(String(a?.uploadId)); },
    channel_media_pause: () => false,
    channel_media_fetch_cancel: () => false,
    channel_media_wake: () => false,
    channel_media_play: () => null,
    channel_media_import: (a) => ({ path: String(a?.src ?? ''), name: String(a?.src ?? '').split(/[\\/]/).pop() ?? '' }),
    channel_uploads: (a) => [...demoUploads.values()].filter((u) => u.community_id === a?.communityId).map((u) => ({ ...u })),
    channel_moderation_queue: (a) => {
      const all = ch(a).queue;
      if (a?.author) {
        const theirs = all.filter((q) => q.author === a.author).sort((x, y) => y.at - x.at);
        return { items: theirs.map((q, i) => ({ ...q, more_from_author: theirs.length - 1 - i })), total: theirs.length };
      }
      // One card per author: their oldest, with how many more wait.
      const cards = new Map<string, QueueItem>();
      for (const q of [...all].sort((x, y) => x.at - y.at)) {
        const c = cards.get(q.author);
        if (c) c.more_from_author += 1; else cards.set(q.author, { ...q, more_from_author: 0 });
      }
      return { items: [...cards.values()].sort((x, y) => y.at - x.at), total: all.length };
    },
    channel_moderation_mod: (a) => {
      const c = ch(a);
      const act = a?.action as ModAction;
      if (act.t === 'approve' || act.t === 'reject') c.queue = c.queue.filter((q) => q.message_id !== act.target);
      if (act.t === 'spam') c.queue = c.queue.filter((q) => q.author !== act.author);
      if (act.t === 'approve' || act.t === 'reject') {
        const p = c.posts.find((x) => x.id === act.target);
        if (p) p.review = act.t === 'approve' ? 'approved' : 'rejected';
      }
      if (act.t === 'hide') act.targets.forEach((t) => { const x = findItem(c, t); if (x) x.hidden = true; });
      c.log.unshift({ at: now(), by: ME, action: act.t === 'spam' ? 'ban' : act.t, target: 'target' in act ? String(act.target ?? '') : 'who' in act ? act.who : '', detail: '{}' });
      emit('channel.queue', { channel: c.view.id, pending: c.queue.length });
    },
    channel_moderation_log: (a) => ({ items: ch(a).log }),
    channel_moderation_settings: (a) => { const c = ch(a); return { post_mode: c.post_mode, comments: c.comments, premod: c.premod, schedule: c.schedule, join: c.view.join, history: c.history }; },
    channel_moderation_settings_set: (a) => {
      const c = ch(a);
      const s = a?.settings as ModSettings;
      if (s.post_mode) c.post_mode = s.post_mode;
      if (s.comments) c.comments = s.comments;
      if (s.premod) c.premod = s.premod;
      if (s.join) c.view.join = s.join;
      if (s.history) c.history = s.history;
      c.schedule = s.schedule && s.schedule.days.length ? s.schedule : undefined;
      c.log.unshift({ at: now(), by: ME, action: 'settings', target: '', detail: JSON.stringify(s) });
      updated(c);
      return s;
    },
    channel_moderation_inherit: () => 0,
    channel_moderation_invite_create: (a) => mocks.channel_invite(a),
    channel_moderation_invite_list: () => [],
    channel_moderation_invite_revoke: () => undefined,
    channel_moderation_invite_status: () => ({ open: 1, max_open: 50, of_channel: 1, max_per_channel: 20, max_per_person: 3 }),
    channel_moderation_blocked: (a) => [...ch(a).banned],
    channel_moderation_requests: (a) => ch(a).requests,
    channel_moderation_request_act: (a) => { const c = ch(a); c.requests = c.requests.filter((r) => r.requester !== a?.requester); updated(c); },
    channel_moderation_requests_count: (a) => ch(a).requests.length,
    channel_readers: (a): ReadersPage => {
      const c = ch(a);
      if (!['moderator', 'admin', 'owner'].includes(roleOf(c) ?? '')) refuse('forbidden', 'not permitted');
      c.readers ??= demoReaders(c);
      const q = String(a?.query ?? '').trim().toLowerCase();
      if (q && !/^[0-9a-f]{1,64}$/.test(q)) return { readers: [], total: 0, partial: false };
      // The staff (authors and above), the banned and I are not in the list.
      const team = new Set(c.staff.filter((s) => s.role !== 'member').map((s) => s.pubkey));
      const all = c.readers
        .filter((r) => r.pubkey !== ME && !team.has(r.pubkey) && !c.banned.includes(r.pubkey) && r.pubkey.startsWith(q))
        .sort((x, y) => y.since - x.since || (x.pubkey < y.pubkey ? -1 : 1));
      const offset = Number(a?.offset ?? 0);
      const limit = Number(a?.limit ?? 100);
      const levels = (who: string) => (c.staff.find((s) => s.pubkey === who)?.levels ?? []).filter((l) => l !== 0);
      return { readers: all.slice(offset, offset + limit).map((r) => ({ ...r, levels: levels(r.pubkey) })), total: all.length, partial: false };
    },
    channel_keeper_status: (a): KeeperStatus => {
      const c = ch(a);
      const owner = c.view.my_role === 'owner';
      const m = c.keeper_move ?? {};
      const move = { owner, handing: !!m.handing, has_root: owner, taking: m.taking, taking_since: m.since };
      return c.view.kept_here && !m.handing
        ? { kept_here: true, online: true, keeper: c.view.owner, epoch: 14, last_epoch_at: now() - 5 * H, next_wake: now() + 40 * 60, readers: c.view.members, generations: 1, garbage: 3, rotate_asked: false, background: false, keep_on: false, background_running: false, ...move }
        : { kept_here: false, online: false, keeper: c.view.owner, readers: 0, generations: 0, garbage: 0, rotate_asked: false, background: false, keep_on: false, background_running: false, ...move };
    },
    channel_tickets: (a): TicketsView => ({ held: ch(a).tickets ?? 0, max: 50 }),
    channel_tickets_request: (a): TicketsView => {
      const c = ch(a);
      c.tickets = Math.min(50, (c.tickets ?? 0) + Number(a?.n ?? 0));
      updated(c);
      return { held: c.tickets, max: 50 };
    },
    channel_transfer: (a) => {
      const c = ch(a);
      c.transfer = { direction: 'out', peer: String(a?.to ?? ''), status: 'offered', created_at: now() };
      updated(c);
      return detail(c);
    },
    channel_transfer_answer: (a) => {
      const c = ch(a);
      if (a?.accept) {
        c.staff = c.staff.map((s) => (s.pubkey === c.view.owner ? { ...s, role: 'admin' } : s.pubkey === ME ? { ...s, role: 'owner' } : s));
        c.view.owner = ME; c.view.my_role = 'owner';
        c.keeper_move = { taking: 'handover', since: now() };
      }
      c.transfer = undefined;
      updated(c);
      return detail(c);
    },
    channel_keeper_take: (a): KeeperStatus => {
      const c = ch(a);
      if (a?.emergency) { c.keeper_move = {}; c.view.kept_here = true; } else c.keeper_move = { taking: 'handover', since: now() };
      updated(c);
      return mocks.channel_keeper_status(a) as KeeperStatus;
    },
    channel_rotate_now: (a): KeeperStatus => {
      const c = ch(a);
      return { kept_here: true, online: true, keeper: c.view.owner, epoch: 15, pending_epoch: 15, last_epoch_at: now(), next_wake: now() + 60, readers: c.view.members, generations: 1, garbage: 3, rotate_asked: true, background: false, keep_on: false, background_running: false, owner: c.view.my_role === 'owner', handing: false, has_root: c.view.my_role === 'owner' };
    },
    channel_keeper_keep_on: (a): KeeperStatus => ({ ...(mocks.channel_keeper_status(a) as KeeperStatus), keep_on: !!a?.on }),

    // ── Levels and sections (communities-wire §14) ──
    channel_scopes: (a) => scopesFor(ch(a)),
    channel_scope_add: (a) => {
      const c = ch(a);
      const input = a?.input as NewScopeInput;
      const scopes = (c.scopes ??= [{ scope_id: 'everyone', title: '', scheme: 'sd-v1', history: 'open', mine: true }]);
      if (scopes.length >= 16) refuse('scope_limit', 'at most 16 scopes');
      scopes.push({
        scope_id: id().slice(1, 33), title: String(input?.title ?? '').trim(), scheme: input?.scheme ?? 'sd-v1', history: input?.history ?? 'open', mine: true,
        icon: input?.icon, color: input?.color, note: input?.note, hidden: !!input?.hidden, order: scopes.length,
      });
      c.levels ??= [{ id: 0, name: 'Читатели', scopes: ['everyone'], can: { comment: true, discuss: false, post: false }, members: c.view.members }];
      updated(c);
      return detail(c);
    },
    channel_scope_set: (a) => {
      const c = ch(a);
      if (roleOf(c) !== 'owner') refuse('forbidden', 'only the owner');
      const s = (c.scopes ?? []).find((x) => x.scope_id === String(a?.scopeId));
      if (!s) refuse('scope_unknown', 'no such scope');
      const input = (a?.input ?? {}) as ScopeSetInput;
      if (input.hidden && s!.scope_id === 'everyone') refuse('scope_everyone', 'everyone is never hidden');
      if (input.title !== undefined) s!.title = input.title.trim();
      if (input.note !== undefined) s!.note = input.note.trim() || undefined;
      if (input.icon !== undefined) s!.icon = input.icon;
      if (input.color !== undefined) s!.color = input.color < 0 ? undefined : input.color;
      if (input.order !== undefined) s!.order = input.order;
      if (input.hidden !== undefined) s!.hidden = input.hidden;
      if (input.no_photo) s!.photo = undefined;
      updated(c);
      return detail(c);
    },
    channel_scopes_order: (a) => {
      const c = ch(a);
      const ids = (a?.ids as string[]) ?? [];
      (c.scopes ?? []).forEach((s) => { const i = ids.indexOf(s.scope_id); if (i >= 0) s.order = i; });
      updated(c);
      return detail(c);
    },
    channel_scope_photo_set: (a) => {
      const c = ch(a);
      const s = (c.scopes ?? []).find((x) => x.scope_id === String(a?.scopeId));
      if (!s) refuse('scope_unknown', 'no such scope');
      s!.photo = `demo-${id().slice(1, 9)}`;
      updated(c);
      return detail(c);
    },
    // A photo of the demo: a landscape in the section's colour.
    channel_scope_photo: (a) => {
      const s = (ch(a).scopes ?? []).find((x) => x.scope_id === String(a?.scopeId));
      if (!s?.photo) return null;
      const tones: [string, string][] = [['#7066c6', '#3d3580'], ['#4c8bf5', '#24508f'], ['#3bb0d6', '#1e6a82'], ['#2f7f74', '#1d5a52'], ['#4caf6a', '#2a6b3d'], ['#e0a43a', '#8a5a12'], ['#e8743b', '#8f3f16'], ['#e0525e', '#8a2630'], ['#d661a8', '#7f2c5f'], ['#7b8496', '#454b57']];
      const [x, y] = tones[s.color ?? 3] ?? tones[3];
      return picture(x, y, '');
    },
    channel_scope_del: (a) => {
      const c = ch(a);
      const sid = String(a?.scopeId);
      if (sid === 'everyone') refuse('scope_everyone', 'the scope everyone stays');
      c.scopes = (c.scopes ?? []).filter((s) => s.scope_id !== sid);
      (c.levels ?? []).forEach((l) => (l.scopes = l.scopes.filter((s) => s !== sid)));
      c.posts = c.posts.filter((p) => (p as FeedItemViewLv).scope_id !== sid);
      updated(c);
      return detail(c);
    },
    channel_levels: (a) => ch(a).levels ?? [],
    channel_level_set: (a) => {
      const c = ch(a);
      const input = a?.level as LevelInput;
      const name = String(input?.name ?? '').trim();
      if (!name || [...name].length > 40) refuse('level_name', 'a level needs a name of up to 40 characters');
      const levels = (c.levels ??= [{ id: 0, name: 'Читатели', scopes: ['everyone'], can: { comment: true, discuss: false, post: false }, members: c.view.members }]);
      const was = input.id === undefined ? undefined : levels.find((l) => l.id === input.id);
      if (was) Object.assign(was, { name, scopes: was.id === 0 ? ['everyone', ...input.scopes.filter((s) => s !== 'everyone')] : input.scopes, can: { ...input.can } });
      else {
        const free = Array.from({ length: 64 }, (_, i) => i).find((i) => !levels.some((l) => l.id === i));
        if (free === undefined) refuse('level_limit', 'at most 64 levels');
        levels.push({ id: free!, name, scopes: [...input.scopes], can: { ...input.can }, members: 0 });
      }
      updated(c);
      return levels.map((l) => ({ ...l }));
    },
    channel_level_del: (a) => {
      const c = ch(a);
      const lid = Number(a?.id);
      if (lid === 0) refuse('level_last', 'the base level stays');
      c.levels = (c.levels ?? []).filter((l) => l.id !== lid);
      c.staff.forEach((s) => (s.levels = s.levels?.filter((x) => x !== lid)));
      c.my_levels = (c.my_levels ?? []).filter((x) => x !== lid);
      updated(c);
      return c.levels.map((l) => ({ ...l }));
    },
    channel_set_levels: (a) => {
      const c = ch(a);
      const who = String(a?.who);
      const next = [...new Set([0, ...((a?.levels as number[]) ?? [])])].sort((x, y) => x - y);
      let s = c.staff.find((x) => x.pubkey === who);
      if (!s) { s = { pubkey: who, role: 'member', muted: false, trusted: false, since: now() }; c.staff.push(s); }
      const before = new Set(s.levels ?? [0]);
      (c.levels ?? []).forEach((l) => { if (l.id !== 0) l.members += Number(next.includes(l.id)) - Number(before.has(l.id)); });
      s.levels = next;
      updated(c);
      return detail(c);
    },
    channel_level_request: (a) => { ch(a); },
    channel_level_requests: (a) => [...(ch(a).level_requests ?? [])],
    channel_level_request_act: (a) => {
      const c = ch(a);
      const who = String(a?.requester);
      const lid = Number(a?.level);
      c.level_requests = (c.level_requests ?? []).filter((r) => !(r.requester === who && r.level === lid));
      if (a?.grant) {
        const s = c.staff.find((x) => x.pubkey === who);
        mocks.channel_set_levels({ communityId: c.view.id, who, levels: [...(s?.levels ?? [0]), lid] });
      }
    },
  };
  // Requests to the first channel, so the tab of requests has cards.
  const first = data()[hex('c1')];
  // And a file of mine that did not go, so its row offers another try.
  const failed: UploadView = {
    id: id(), community_id: first.view.id, name: 'Отчёт о бэкапах.pdf', post_id: '', status: 'failed', error: 'channel_not_delivered: err.not_delivered',
    done: 900_000, total: 3_100_000, created_at: T0 - 20 * 60,
  };
  demoUploads.set(failed.id, failed);
  first.requests = [
    { community_id: first.view.id, requester: VERA, status: 'pending', note: 'Хочу читать новости первой. Я тестирую Android-сборки.', created_at: T0 - 4 * H },
    { community_id: first.view.id, requester: NIKITA, status: 'pending', note: '', created_at: T0 - 26 * H },
  ];
  demoLevels();
  demoStage3();
  demoOutbox(mocks);
  for (const [name, fn] of Object.entries(mocks)) mockCommand(name, fn);
}

/**
 * Stage 3 in the demo (communities-wire §15–§17), by a knob of the browser
 * (`communities.demo.stage3`, JSON) so that a page can be shown in each of
 * its states: the keeper of «Veydan: новости» on my other device (`keeper`:
 * `elsewhere` | `taking` | `stalled` | `handing`), the channel offered to me
 * (`transfer_in`) or by me (`transfer_out`: `offered` | `declined`), and
 * tickets of a moderator in the photo club (`tickets`: how many).
 */
function demoStage3() {
  let knob: { keeper?: string; transfer_in?: boolean; transfer_out?: 'offered' | 'declined'; tickets?: number; post_level?: boolean } = {};
  try {
    knob = JSON.parse(localStorage.getItem('communities.demo.stage3') ?? '{}');
  } catch { /* no knob: the demo as it is */ }
  const all = data();
  const news = all[hex('c1')];
  if (knob.keeper) {
    news.view.kept_here = knob.keeper === 'handing';
    news.keeper_move = knob.keeper === 'handing' ? { handing: true }
      : knob.keeper === 'taking' ? { taking: 'handover', since: T0 - 60 }
      : knob.keeper === 'stalled' ? { taking: 'handover', since: T0 - 20 * 60 } : {};
  }
  if (knob.transfer_out) news.transfer = { direction: 'out', peer: BORIS, status: knob.transfer_out, created_at: T0 - H };
  if (knob.transfer_in) all[hex('c6')].transfer = { direction: 'in', peer: BORIS, status: 'offered', created_at: T0 - 10 * 60 };
  // A level of the photo club that lets its readers post in its section (2026-10-23).
  if (knob.post_level) {
    const friends = all[hex('c2')].levels?.find((l) => l.id === 1);
    if (friends) friends.can = { ...friends.can, post: true };
  }
  if (knob.tickets !== undefined) {
    const club = all[hex('c2')];
    club.view.my_role = 'moderator';
    club.staff.push({ pubkey: ME, role: 'moderator', muted: false, trusted: false, since: T0 - 5 * D });
    club.tickets = knob.tickets;
  }
}

/**
 * What I write on its way, in the demo, by a knob of the browser
 * (`communities.demo.outbox`, JSON): a post of mine in «Veydan: новости»
 * still waiting in the outbox (`pending`), one the outbox gave up on
 * (`failed`, told as `channel.outbox` when the feed is first read),
 * commands that answer late (`slow`, ms), so that a post or a comment is
 * seen before its command answers, an album of three pictures sent
 * together (`album`), the states of a card (`cards`), the module opening late (`late`, ms), a feed read
 * slowly (`slow_history`, ms) and a long feed over the sections (`many`: how many posts more), to time
 * a switch of sections on.
 */
/** A frame of a demo video, as a sender's preview: a base64 JPEG drawn here (none without a canvas). */
function demoFrame(w: number, h: number, a: string, b: string): string | undefined {
  try {
    const c = document.createElement('canvas');
    c.width = w; c.height = h;
    const g = c.getContext('2d');
    if (!g) return undefined;
    const grad = g.createLinearGradient(0, 0, w, h);
    grad.addColorStop(0, a); grad.addColorStop(1, b);
    g.fillStyle = grad; g.fillRect(0, 0, w, h);
    g.fillStyle = 'rgba(255,255,255,0.85)';
    g.beginPath(); g.arc(w * 0.7, h * 0.35, Math.min(w, h) * 0.12, 0, Math.PI * 2); g.fill();
    g.fillStyle = 'rgba(0,0,0,0.35)';
    g.beginPath(); g.moveTo(0, h); g.lineTo(w * 0.35, h * 0.55); g.lineTo(w * 0.6, h * 0.8); g.lineTo(w * 0.8, h * 0.6); g.lineTo(w, h); g.fill();
    const url = c.toDataURL('image/jpeg', 0.7);
    return url.startsWith('data:image/jpeg') ? url.slice(url.indexOf(',') + 1) : undefined;
  } catch { return undefined; }
}

/**
 * Videos of a post (`videos`: the URL of a real video the demo plays, served beside it): one with the
 * frame its sender took, not fetched; one of mine without a frame (sent before senders took it), here,
 * whose frame is taken by the page; an album of two pictures and a video; a comment with a video.
 */
function demoVideos(mocks: Record<string, (a?: Record<string, unknown>) => unknown>, news: DemoChannel, url: string) {
  const cid = news.view.id;
  const video = (name: string, size: number, dim?: [number, number], thumb?: string, batch?: string) =>
    JSON.stringify({ v: 1, t: 'media', kind: 'video', name, mime: 'video/mp4', size, dim, thumb, batch });
  const framed = item(cid, MARINA, now() - 900, 'Просто видео в тему', {
    media_json: video('207.mp4', 100_034_601, [1280, 720], demoFrame(320, 180, '#f2994a', '#5b2a86')), reactions: [{ emoji: '🔥', count: 7, mine: false }], comments: 1,
  });
  const mine = item(cid, ME, now() - 800, 'Старое видео без кадра — кадр берёт страница', { media_json: video('clip.mp4', 2_400_000, undefined) });
  news.posts.push(framed, mine);
  const batch = 'demo-album-video';
  const parts: [string, string | undefined][] = [['walk-1.jpg', undefined], ['walk-2.jpg', undefined], ['walk.mp4', demoFrame(320, 180, '#27ae60', '#1f3b73')]];
  parts.forEach(([name, thumb], i) => {
    const last = i === parts.length - 1;
    news.posts.push(item(cid, ALICE, now() - 700 + i, last ? 'Прогулка: два фото и видео' : undefined, {
      media_json: name.endsWith('.mp4') ? video(name, 18_400_000, [1280, 720], thumb, batch) : media(name, 'image', 380_000, undefined, [960, 640], batch),
      ...(last ? { comments: 0 } : {}),
    }));
  });
  news.posts.sort((a, b) => a.created_at - b.created_at);
  const comment = item(cid, BORIS, now() - 600, 'Вот как это выглядит у меня', { post_id: framed.id, media_json: video('reply.mp4', 1_800_000, [720, 1280], demoFrame(180, 320, '#56ccf2', '#2d3436')) });
  const here = new Set<string>([mine.id]);
  const fetchIt = mocks.channel_media_fetch;
  mocks.channel_media_fetch = (a) => {
    here.add(String(a?.messageId));
    return new Promise((ok) => setTimeout(() => ok(fetchIt(a)), 600));
  };
  // Mine and fetched ones play from the server of files (here: the URL given).
  mocks.channel_media_play = (a) => (here.has(String(a?.messageId)) ? url : null);
  (news.comments_of[framed.id] ??= []).push(comment);
}

function demoOutbox(mocks: Record<string, (a?: Record<string, unknown>) => unknown>) {
  let knob: { pending?: boolean; failed?: boolean; slow?: number; album?: boolean; cards?: boolean; late?: number; slow_history?: number; many?: number; videos?: string; recordings?: boolean } = {};
  try {
    knob = JSON.parse(localStorage.getItem('communities.demo.outbox') ?? '{}');
  } catch { /* no knob: the demo as it is */ }
  const news = data()[hex('c1')];
  if (knob.album) {
    const batch = 'demo-album-0001';
    const caption = 'Фото со встречи тестировщиков в субботу.';
    ['meetup-1.jpg', 'meetup-2.jpg', 'meetup-3.jpg'].forEach((name, i, all) => {
      const last = i === all.length - 1;
      news.posts.push(item(news.view.id, ALICE, now() - 90 + i, last ? caption : undefined, {
        media_json: media(name, 'image', 380_000, last ? caption : undefined, [960, 640], batch),
        ...(last ? { reactions: [{ emoji: '❤️', count: 12, mine: false }], comments: 3 } : {}),
      }));
    });
  }
  // The states of a card of the redesign (2026-10-23): words cut at three lines, Markdown cut, a picture
  // without words, an album with files and a Markdown caption, an edited post (`edited_at`, the core's stage 1).
  if (knob.cards) {
    const cid = news.view.id;
    const long = 'Как устроены разделы канала\nРаздел — это отдельная лента внутри канала со своим ключом. Читатель видит только те разделы, которые открывает его уровень. Владелец сам задаёт название, значок и цвет, а ещё может сделать раздел скрытым: тогда о нём не узнают те, кому он закрыт. Порядок плиток тоже настраивается.';
    const plan = '## План на неделю\n\nСначала — **разделы** и их плитки, потом редактор с предпросмотром.\n\n- понедельник: карточка поста\n- вторник: страница поста\n- среда: разделы\n\n> Пятница — день проверки на телефонах.';
    const caption = 'Фото со встречи тестировщиков\n\nСпасибо всем, кто пришёл! **Девять фото**, программа и запись доклада про синхронизацию.';
    const batch = 'demo-album-cards';
    news.posts.push(
      item(cid, ALICE, now() - 600, long, { reactions: [{ emoji: '👍', count: 48, mine: false }], comments: 8, edited_at: now() - 120 } as Partial<FeedItemViewLv>),
      item(cid, ME, now() - 500, plan, { fmt: 'md', rich: demoRich(plan), comments: 2 }),
      item(cid, MARINA, now() - 400, undefined, { media_json: media('sunset.jpg', 'image', 520_000, undefined, [960, 640]), reactions: [{ emoji: '❤️', count: 21, mine: true }], comments: 5 }),
      item(cid, BORIS, now() - 380, 'Ссылка на сторонний канал с раздачей ключей', { hidden: true }),
      item(cid, NIKITA, now() - 360, 'Удалённый пост', { deleted: true }),
    );
    const parts: [string, PostMedia['kind']][] = [['meetup-1.jpg', 'image'], ['meetup-2.jpg', 'image'], ['meetup-3.jpg', 'image'], ['Программа встречи.pdf', 'file'], ['Запись доклада.m4a', 'audio']];
    parts.forEach(([name, kind], i) => {
      const last = i === parts.length - 1;
      news.posts.push(item(cid, ME, now() - 300 + i, last ? caption : undefined, {
        media_json: media(name, kind, kind === 'image' ? 380_000 : 18_400_000, last ? caption : undefined, kind === 'image' ? [960, 640] : undefined, batch),
        ...(last ? { fmt: 'md', rich: demoRich(caption), reactions: [{ emoji: '🎉', count: 12, mine: false }], comments: 3 } : {}),
      }));
    });
  }
  // Voice messages and circles (`recordings`, 2026-10-23): a voice post, a circle post, and under the voice post a voice
  // comment and a circle comment, a reader's among them.
  if (knob.recordings) {
    const cid = news.view.id;
    const voice = item(cid, ALICE, now() - 260, undefined, { media_json: recMedia('voice', 52_000, 14_000, demoWave(3)), reactions: [{ emoji: '🔥', count: 7, mine: false }], comments: 2 });
    const circle = item(cid, MARINA, now() - 200, undefined, { media_json: recMedia('circle', 1_400_000, 9_000), reactions: [{ emoji: '👋', count: 15, mine: true }], comments: 0 });
    news.posts.push(voice, circle);
    news.comments_of[voice.id] = [
      item(cid, BORIS, now() - 180, undefined, { post_id: voice.id, media_json: recMedia('voice', 21_000, 6_000, demoWave(11)), reactions: [{ emoji: '👍', count: 2, mine: false }] }),
      item(cid, NIKITA, now() - 120, undefined, { post_id: voice.id, media_json: recMedia('circle', 900_000, 5_000) }),
    ];
  }
  // A long feed (`many`): words of three lines and more, Markdown, pictures, in everyone's and the two sections I am in.
  if (knob.many) {
    const cid = news.view.id;
    const into = [undefined, 'a1'.repeat(16), 'b2'.repeat(16)];
    const who = [ALICE, BORIS, MARINA, NIKITA, ME];
    const md = '## Заметка\n\nСписок дел на неделю, **важное** выделено:\n\n- разделы и плитки\n- редактор поста\n- проверка на телефоне\n\n> Пятница — день проверки.';
    const words = 'Длинный пост ленты\nРаздел — это отдельная лента внутри канала со своим ключом. Читатель видит только те разделы, которые открывает его уровень. Владелец сам задаёт название, значок и цвет, а ещё может сделать раздел скрытым.';
    for (let i = 0; i < knob.many; i++) {
      const at = now() - 7200 - (knob.many - i) * 600;
      const extra: Partial<FeedItemViewLv> = { scope_id: into[i % 3], reactions: i % 2 ? [{ emoji: '👍', count: i, mine: false }] : [], comments: i % 4 };
      if (i % 5 === 0) extra.media_json = media(`many-${i}.jpg`, 'image', 380_000, undefined, [960, 640]);
      news.posts.push(i % 3 === 1 ? item(cid, who[i % 5], at, md, { ...extra, fmt: 'md', rich: demoRich(md) }) : item(cid, who[i % 5], at, `${i}. ${words}`, extra));
    }
    news.posts.sort((a, b) => a.created_at - b.created_at);
  }
  if (knob.videos) demoVideos(mocks, news, knob.videos);
  if (knob.pending) news.posts.push({ ...item(news.view.id, ME, now() - 40, 'Анонс встречи в субботу: место и время — в комментариях.'), pending: true } as FeedItemView);
  let failedId: string | null = null;
  if (knob.failed) {
    const x = item(news.view.id, ME, now() - 25, 'Обновлённый список вопросов к релизу 5.3.');
    news.posts.push(x);
    failedId = x.id;
    const history = mocks.channel_history;
    let told = false;
    mocks.channel_history = (a) => {
      if (!told && failedId && String(a?.communityId) === news.view.id) {
        told = true;
        setTimeout(() => emit('channel.outbox', { community_id: news.view.id, ref_id: failedId, state: 'failed', error: 'channel_not_delivered: the relays did not take it' }), 300);
      }
      return history(a);
    };
  }
  mocks.channel_outbox_retry = (a) => {
    const c = ch(a);
    if (failedId && c === news) {
      const ref = failedId;
      failedId = null;
      emit('channel.outbox', { community_id: c.view.id, ref_id: ref, state: 'queued' });
      setTimeout(() => emit('channel.outbox', { community_id: c.view.id, ref_id: ref, state: 'sent' }), 1200);
    }
    return detail(c);
  };
  // The module opening after the page (`late`, ms): the rows are refused as before `channel.ready`, which comes then.
  if (knob.late) {
    const until = Date.now() + knob.late;
    const list = mocks.channels_list;
    mocks.channels_list = (a) => (Date.now() < until ? refuse('no_session', 'no identity at work') : list(a));
    setTimeout(() => emit('channel.ready', {}), knob.late);
  }
  // A feed read slowly (`slow_history`, ms): a channel opened again shows what was read meanwhile.
  if (knob.slow_history) {
    const history = mocks.channel_history;
    const ms = knob.slow_history;
    mocks.channel_history = (a) => new Promise((ok) => setTimeout(() => ok(history(a)), ms));
  }
  const slow = knob.slow ?? 0;
  if (slow > 0) {
    for (const name of ['channel_post', 'channel_comment', 'channel_react', 'channel_media_send']) {
      const fn = mocks[name];
      mocks[name] = (a) => new Promise((ok) => setTimeout(() => ok(fn(a)), slow));
    }
  }
}
