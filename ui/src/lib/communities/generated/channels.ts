// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Generated from Rust (veydan-communities/src/bindings.rs). Do not edit:
// run `make msg-types` after changing the Rust types.

import type { MdBlock } from '$lib/messenger/seams/shared';

/** The newest post of a channel, for its row in the list of chats. */
export type LastPost = { id: string, 
/**
 * Hex pubkey.
 */
author: string, 
/**
 * None for a file without a caption or a post that is gone.
 */
text?: string, has_media: boolean, 
/**
 * A recording made in the app: `voice` or `circle` (the row says it by its kind); absent for any other file.
 */
media_kind?: string, at: number, };

/** A channel as `channels_list` lists it: the row of the list of chats under `chat_id` (`channel:<id>`), with what the user chose of it in the messenger. */
export type ChannelView = { 
/**
 * The id of the community (64 hex).
 */
id: string, 
/**
 * `channel:<id>`: the chat whose pin, mute and archive the messenger keeps.
 */
chat_id: string, name: string, about: string, 
/**
 * The address of the cover on a host of the channel.
 */
picture?: string, 
/**
 * Hex pubkey of the owner.
 */
owner: string, 
/**
 * `open` | `request` | `invite`
 */
join: string, 
/**
 * `none` | `joining` | `requested` | `joined` | `rejected` | `out` | `disbanded`
 */
membership: string, 
/**
 * `owner` | `admin` | `moderator` | `author` | `member`; none for a reader.
 */
my_role?: string, 
/**
 * Readers by the newest snapshot of the keeper.
 */
members: number, relays: Array<string>, storage: Array<string>, link_epoch: number, 
/**
 * This device keeps the channel (its keeper runs here).
 */
kept_here: boolean, 
/**
 * Something of the channel is newer than this build reads.
 */
needs_update: boolean, 
/**
 * Why I am out: `left` | `removed` | `banned` | `disbanded`; none while in.
 */
out_reason?: string, 
/**
 * When the keeper was last heard of by every reader: the time of the
 * newest key (epoch) it handed out, unix seconds.
 */
keeper_last_seen?: number, unread: number, 
/**
 * What the user chose of the chat `chat_id`, kept by the messenger
 * (`messenger_chat_set_pinned`, `…_muted`, `…_archived`).
 */
pinned: boolean, muted: boolean, archived: boolean, last?: LastPost, 
/**
 * What the list of chats orders the row by and shows as its time: the
 * date of `last`, or, with no post yet, when the channel came here
 * (`created_at`). Opening, reading, comments, reactions, edits and the
 * news of the journal never move it (owner 2026-10-23).
 */
last_at: number, created_at: number, updated_at: number, };

/** When readers may post: days 1 (Monday) to 7, `from` and `to` as HH:MM in `tz`; no days takes the schedule away. */
export type ScheduleView = { tz: string, days: Array<number>, from: string, to: string, };

/** A section of the channel: a scope of its own keys (`everyone` is the base one, its title empty). */
export type ScopeView = { 
/**
 * `everyone`, or 32 hex.
 */
scope_id: string, 
/**
 * Sealed, read by members; empty for `everyone` unless the owner gave
 * it one (the page names it).
 */
title: string, 
/**
 * `sd-v1` | `flat-v1`
 */
scheme: "sd-v1" | "flat-v1", 
/**
 * `open` | `from_join`
 */
history: "open" | "from_join", 
/**
 * I belong to it: a level of mine gives it, or I am a moderator or above.
 */
mine: boolean, 
/**
 * I may post into it (2026-10-23): an author and above where I belong,
 * a reader where a level of mine that lets post gives it.
 */
post?: boolean, 
/**
 * The look of the tile (2026-10-23), from the sealed meta: a key of
 * the app's set of icons.
 */
icon?: string, 
/**
 * The section has a photo: its id (the hash of its blob), to cache by;
 * `channel_scope_photo` gives the picture.
 */
photo?: string, 
/**
 * One of the ten colours of the palette, 0…9.
 */
color?: number, 
/**
 * Where the owner put it; the list comes sorted by it already.
 */
order?: number, 
/**
 * The line under the title on the tile.
 */
note?: string, 
/**
 * Hidden: only its members are shown it (and they are, with this
 * mark); its title and look are sealed in its own feed. Always given.
 */
hidden?: boolean, 
/**
 * Posts in it not read yet, by the rule of the channel's count.
 * Always given.
 */
unread?: number, };

/** What a level lets do beyond reading its sections. */
export type LevelRights = { comment: boolean, discuss: boolean, post: boolean, };

/** A level: a name the owner gives, the sections it opens and its rights. Id 0 is the base level of everyone in. */
export type LevelView = { id: number, name: string, 
/**
 * The sections it opens; left out: `everyone`, and a section shown
 * me not (`levels::scope_shown`: not even the id of a hidden one).
 */
scopes: Array<string>, can: LevelRights, 
/**
 * How many people hold it by the journal (everybody in, for the base
 * level: the readers the last snapshot counted).
 */
members: number, };

/** A reader asks for a level (`channel.level_request`), for the moderators. */
export type LevelRequestView = { 
/**
 * Hex pubkey.
 */
requester: string, level: number, note: string, created_at: number, };

/** `channel_scope_add`: a new section, its look, hidden or not. */
export type NewScopeInput = { title: string, 
/**
 * `sd-v1` (default) | `flat-v1`
 */
scheme?: "sd-v1" | "flat-v1", 
/**
 * `open` (default) | `from_join`
 */
history?: "open" | "from_join", 
/**
 * A key of the app's set of icons, `[a-z0-9-]{1,32}`.
 */
icon?: string, 
/**
 * 0…9
 */
color?: number, order?: number, 
/**
 * The line under the title on the tile, ≤ 80 characters.
 */
note?: string, hidden?: boolean, };

/** `channel_scope_set`: what changes of a section (the owner); a field left out stays. */
export type ScopeSetInput = { title?: string, icon?: string, 
/**
 * 0…9, or -1 for none.
 */
color?: number, order?: number, note?: string, hidden?: boolean, no_photo?: boolean, };

/** `channel_level_set`: a level made (no `id`) or changed. */
export type LevelInput = { id?: number, name: string, scopes: Array<string>, can: LevelRights, };

/** Somebody the journal names: staff, or a reader it mutes or trusts. */
export type StaffView = { 
/**
 * Hex pubkey.
 */
pubkey: string, 
/**
 * `owner` | `admin` | `moderator` | `author` | `member`
 */
role: string, muted: boolean, muted_until?: number, trusted: boolean, since: number, 
/**
 * The levels `set_levels` gave (ids; the base level 0 is everybody's
 * and left out); always given, optional to a page made before them.
 */
levels?: Array<number>, 
/**
 * Uploads into his own quota, not the owner's (`set_quota`,
 * communities-wire §9.2): `true`; left out in the owner's.
 */
quota_own?: boolean, };

/** A handover of the channel to a new owner (communities-wire §16): offered to me (`in`) or by me (`out`). */
export type TransferView = { 
/**
 * `in`: offered to me, «Вам передают канал»; `out`: offered by me.
 */
direction: "in" | "out", 
/**
 * The owner who offers it (`in`), the one offered it (`out`), hex.
 */
peer: string, 
/**
 * `offered` | `declined`
 */
status: "offered" | "declined", created_at: number, };

/** What of mine in a channel still waits for the relays; `channel_outbox_retry` tries it now, `channel.outbox` tells of each row. */
export type OutboxView = { queued: number, failed: number, error?: string, };

/** A channel as `channel_get` and the commands that change it answer: the row, the rules, the staff, whether a key is here, the link to hand on. */
export type ChannelDetail = { channel: ChannelView, 
/**
 * `open` | `from_join`: how much of the past a newcomer reads.
 */
history: string, 
/**
 * `all` | `authors`: who posts.
 */
post_mode: string, 
/**
 * `on` | `off`
 */
comments: string, 
/**
 * `off` | `newcomers` | `all`
 */
premod: string, schedule?: ScheduleView, 
/**
 * Ids of the pinned posts.
 */
pinned: Array<string>, staff: Array<StaffView>, disbanded: boolean, 
/**
 * A key of the channel is here; without one the page says «waiting
 * for the keeper's key».
 */
has_key: boolean, 
/**
 * Posts and comments here that wait for a key newer than the one
 * held: with `has_key` the page says «the key is old, catching up».
 */
waiting: number, 
/**
 * The keeper of the current series, hex pubkey.
 */
keeper?: string, 
/**
 * The link to hand on (with its secret when this device holds it).
 */
link?: string, 
/**
 * The sections of the channel, `everyone` first (communities-wire §14).
 * These three are always given, optional to a page made before them.
 */
scopes?: Array<ScopeView>, 
/**
 * The levels, by id; 0 is the base level of everyone in.
 */
levels?: Array<LevelView>, 
/**
 * The levels I hold (the base level 0 left out).
 */
my_levels?: Array<number>, 
/**
 * The channel handed to a new owner (wire §16): an offer to me not
 * answered yet, or mine not taken yet.
 */
transfer?: TransferView, 
/**
 * What of mine in this channel still waits for the relays (always
 * given; optional to a page made before it).
 */
outbox?: OutboxView, };

/** What `channel_create` takes: a name, how readers get in, the relays and the blob hosts of the channel. */
export type NewChannelInput = { name: string, about?: string, 
/**
 * `open` | `request` | `invite`
 */
join: string, 
/**
 * `open` | `from_join`; `open` when not given.
 */
history?: string, 
/**
 * The relays of the channel: relays of the messenger's pool (those of
 * the home set, as `messenger_relays_list` gives them), at least one.
 */
relays: Array<string>, 
/**
 * The blob hosts of the channel (`storage[]`): at least one.
 */
storage: Array<string>, };

/** An operation of the journal as `channel_act` takes it, tagged by `op`; `leave` leaves the channel. */
export type ChannelAction = { "op": "settings", name?: string, about?: string, 
/**
 * `open` | `from_join`
 */
history?: string, relays?: Array<string>, storage?: Array<string>, 
/**
 * `all` | `authors`
 */
post_mode?: string, 
/**
 * `off` | `newcomers` | `all`
 */
premod?: string, schedule?: ScheduleView, 
/**
 * `on` | `off`
 */
comments?: string, 
/**
 * `open` | `request` | `invite`
 */
join?: string, } | { "op": "set_role", who: string, role: string, } | { "op": "ban", who: string, 
/**
 * `spam` or nothing.
 */
reason?: string, } | { "op": "unban", who: string, reason?: string, } | { "op": "remove", who: string, } | { "op": "set_muted", who: string, muted: boolean, until?: number, } | { "op": "set_trusted", who: string, trusted: boolean, } | { "op": "set_quota", who: string, own: boolean, } | { "op": "rotate_now", scope_id?: string, } | { "op": "pin", post_id: string, pinned: boolean, } | { "op": "transfer_ownership", to: string, } | { "op": "leave" } | { "op": "disband" };

/** How many set one reaction, and whether I am one of them. */
export type ReactionCount = { emoji: string, count: number, 
/**
 * I am one of those who set it (on any of my devices).
 */
mine: boolean, };

/** A post or a comment as a page shows it. */
export type FeedItemView = { id: string, community_id: string, 
/**
 * Of a comment: its post. Empty for a post.
 */
post_id: string, parent_id?: string, 
/**
 * Hex pubkey.
 */
author: string, created_at: number, text?: string, 
/**
 * `md` when `text` is written in Markdown; absent for plain text.
 */
fmt?: string, 
/**
 * Of a Markdown `text`: its blocks, rendered as a chat's message is
 * (MessageContent); absent when plain, or too long to be Markdown.
 */
rich?: Array<MdBlock>, 
/**
 * The media envelope (`t: media`) as JSON, when it is a file.
 */
media_json?: string, 
/**
 * `ok` | `pending` | `approved` | `rejected` | `released`
 */
review: string, hidden: boolean, deleted: boolean, pinned: boolean, reactions: Array<ReactionCount>, 
/**
 * Of a post: how many comments are here.
 */
comments: number, 
/**
 * The section it is in (`everyone`, or the id of a scope); always
 * given, optional to a page made before the sections.
 */
scope_id?: string, 
/**
 * When it was edited last (the edit's own date, unix seconds); absent
 * for a message never edited. Of a file's post, an edit is its caption.
 */
edited_at?: number, 
/**
 * Mine, and no relay has it yet: shown as on its way until
 * `channel.outbox` says it went (or failed).
 */
pending: boolean, };

/** Where the next page starts: `next` of the page before, passed back as it came. */
export type FeedCursorView = { created_at: number, id: string, };

/** A page of `channel_history` or `channel_thread`; `asked`: the relays were asked for more, which comes as `channel.message`. */
export type FeedPageView = { items: Array<FeedItemView>, 
/**
 * The date of `next` alone, for a page that still pages by `before`.
 */
next_before?: number, 
/**
 * What to pass as `next` for the next page.
 */
next?: FeedCursorView, asked: boolean, };

/** A file of mine on its way to the hosts of a channel (`channel_media_send`, `channel_uploads`); its progress comes as `channel.media`. */
export type UploadView = { id: string, community_id: string, 
/**
 * The name of the file, as the page shows it while it goes.
 */
name: string, 
/**
 * The post a comment goes under; empty for a post.
 */
post_id: string, parent_id?: string, caption?: string, 
/**
 * `uploading` | `paused` | `failed`
 */
status: string, error?: string, done: number, total: number, created_at: number, 
/**
 * A recording made in the app: `voice` or `circle` (`channel_media_record`); absent for a file picked.
 */
kind?: string, };

/** A file picked to be sent, a file of the device now (`channel_media_import`); `url`: where the page reads a video for its frame. */
export type ImportedFile = { 
/**
 * What `channel_media_send` sends.
 */
path: string, 
/**
 * The name the user picked.
 */
name: string, 
/**
 * A video the webview plays, from the messenger's server of files;
 * `null` for anything else, or without the messenger.
 */
url?: string, };

/** The card of a channel from its link, before joining (`channel_preview_link`). */
export type ChannelPreview = { community_id: string, name: string, about: string, 
/**
 * The cover's address on a host of the channel.
 */
picture?: string, 
/**
 * Readers by the last snapshot: an estimate.
 */
members: number, 
/**
 * `open` | `request` | `invite`.
 */
join: string, 
/**
 * Hex pubkey of the owner, whose signature the card carries.
 */
owner: string, 
/**
 * The relay of the link the card came from.
 */
relay: string, 
/**
 * The link carries the secret of the channel: it lets in at once.
 */
has_secret: boolean, 
/**
 * The link names an invitation.
 */
invited: boolean, 
/**
 * Where I stand, when the channel is here already (`joined`, `requested`, …).
 */
membership?: string, };

/** Where the files of a new channel of mine live (`channel_storage_plan`): the free public media of the messenger without Veydan Cloud. */
export type StoragePlanView = { 
/**
 * The account has no Veydan Cloud that keeps files longer than the
 * free media: the files of its channels go away after `retention_days`.
 */
free: boolean, 
/**
 * How long the media of the account keeps a file, days; 0: no limit.
 */
retention_days: number, };

/** An invitation to a channel, one that came to me (`in`) or one I sent (`out`). */
export type InviteView = { invite_id: string, community_id: string, 
/**
 * `in` | `out`
 */
direction: string, 
/**
 * The other side, hex pubkey: who invited me, or whom I invited.
 */
peer: string, 
/**
 * `sent` | `accepted` | `declined` | `done` | `expired` | `cancelled`
 */
status: string, 
/**
 * The name of the channel as the invitation carries it.
 */
name: string, about: string, picture?: string, created_at: number, expires_at: number, 
/**
 * Of one that came: the journal here bears out that its sender is a
 * moderator; until then the page says it is not checked yet.
 */
verified: boolean, };

/** A request to be let into a channel, waiting for a moderator. */
export type RequestView = { community_id: string, 
/**
 * Hex pubkey of who asks.
 */
requester: string, 
/**
 * `pending` | `approved` | `rejected`
 */
status: string, 
/**
 * What they wrote with it.
 */
note: string, created_at: number, };
