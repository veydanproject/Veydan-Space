// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Generated from Rust (veydan-communities/src/bindings.rs). Do not edit:
// run `make msg-types` after changing the Rust types.

import type { ScheduleView } from './channels';

/** A message waiting for a moderator: one line per author, or each of one author's when the queue is asked for theirs. */
export type QueueItem = { message_id: string, 
/**
 * Hex pubkey.
 */
author: string, text?: string, at: number, 
/**
 * The version an approval names.
 */
version: string, 
/**
 * How many more of the same author wait behind it.
 */
more_from_author: number, };

/** A page of the queue, as `channel_moderation_queue` answers (`author`: that author's messages alone). */
export type QueuePage = { items: Array<QueueItem>, next_before?: number, 
/**
 * How many messages wait (of the author asked for, when one was).
 */
total: number, };

/** A reader of a channel as its team sees them (`channel_readers`); the journal does not list readers, the keeper's rows or its snapshot do. */
export type ReaderView = { 
/**
 * Hex pubkey.
 */
pubkey: string, 
/**
 * When the keeper let them in.
 */
since: number, 
/**
 * The levels the journal gives (ids; the base level 0 is everybody's
 * and left out), as `StaffView::levels`.
 */
levels: Array<number>, };

/** A page of the readers, as `channel_readers` answers: newest first, the staff, the banned and me left out; `partial`: shards of the snapshot still on their way. */
export type ReadersPage = { readers: Array<ReaderView>, 
/**
 * How many readers match the query, on every page.
 */
total: number, 
/**
 * Some shards of the snapshot are not on this device yet (asked of
 * the hosts): readers may be missing, and asked again soon the list
 * may hold them.
 */
partial: boolean, };

/** One act of moderation, as `channel_moderation_mod` takes it; tagged by `t`. */
export type ModAction = { "t": "hide", targets: Array<string>, } | { "t": "hide_author", author: string, from: number, to: number, } | { "t": "unhide", targets: Array<string>, } | { "t": "approve", target: string, version: string, } | { "t": "reject", target: string, } | { "t": "spam", author: string, target?: string, } | { "t": "mute", who: string, until?: number, } | { "t": "unmute", who: string, } | { "t": "trust", who: string, trusted: boolean, };

/** One line of the log of moderation. */
export type LogEntry = { at: number, 
/**
 * Hex pubkey of who acted.
 */
by: string, 
/**
 * `hide` | `unhide` | `approve` | `reject` | `ban` | `unban` | `mute`
 * | `unmute` | `trust` | `untrust` | `remove` | `pin` | `unpin` | `settings`
 */
action: string, 
/**
 * The message, the post or the person; empty for settings.
 */
target: string, 
/**
 * JSON with what else the line says.
 */
detail: string, 
/**
 * Of a line about a post or a comment here: the start of its words, so
 * the log names it as a reader would; none for a person, the rules, or
 * a message gone from this device.
 */
excerpt?: string, };

/** A page of the log, as `channel_moderation_log` answers. */
export type LogPage = { items: Array<LogEntry>, next_before?: number, };

/** The rules of a channel, as `channel_moderation_settings` answers and `channel_moderation_settings_set` takes them; a field left out stays as it is. */
export type ModSettings = { 
/**
 * `authors`, always in stage 1 (wire §3, 2026-10-22); one sent is not
 * applied.
 */
post_mode?: string, 
/**
 * `on` | `off`
 */
comments?: string, 
/**
 * `off` | `newcomers` | `all`
 */
premod?: string, 
/**
 * A schedule without days takes the schedule away.
 */
schedule?: ScheduleView, 
/**
 * `open` | `request` | `invite`
 */
join?: string, 
/**
 * `open` | `from_join`
 */
history?: string, };

/** The invitations open now against their limits (`channel_moderation_invite_status`). */
export type InviteStatusView = { 
/**
 * Open invitations of mine, of every channel.
 */
open: number, max_open: number, 
/**
 * Open invitations of this channel.
 */
of_channel: number, max_per_channel: number, max_per_person: number, };

/** The tickets a moderator holds, as `channel_tickets` and `channel_tickets_request` answer. */
export type TicketsView = { 
/**
 * Tickets held and good for a while yet.
 */
held: number, 
/**
 * The most a moderator holds at once.
 */
max: number, };

/** The keeper of a channel, as `channel_keeper_status` and `channel_rotate_now` answer. */
export type KeeperStatus = { 
/**
 * This device keeps the channel; the rest is known only then.
 */
kept_here: boolean, 
/**
 * The keeper runs: the module is on and has a session.
 */
online: boolean, 
/**
 * The keeper of the current series, hex pubkey.
 */
keeper?: string, 
/**
 * The newest epoch published.
 */
epoch?: number, 
/**
 * An epoch built and still waiting for its hosts or its relays.
 */
pending_epoch?: number, 
/**
 * When the newest epoch was built, unix seconds.
 */
last_epoch_at?: number, 
/**
 * When the cycle runs next, unix seconds.
 */
next_wake?: number, 
/**
 * Readers the keeper holds.
 */
readers: number, 
/**
 * Live generations of the tree (two during a migration).
 */
generations: number, 
/**
 * Blobs waiting to be removed from the hosts.
 */
garbage: number, 
/**
 * An epoch asked for now (`channel_rotate_now`) and not built yet.
 */
rotate_asked: boolean, 
/**
 * The phone runs the keeper with the app off the screen (Android):
 * the owner's switch is shown. False on a desktop, whose keeper runs
 * while the app does.
 */
background: boolean, 
/**
 * The owner keeps the keeper of this channel on (`channel_keeper_keep_on`).
 */
keep_on: boolean, 
/**
 * The keeper's service of the phone runs now: «работает в фоне».
 */
background_running: boolean, 
/**
 * I am the owner: «Сделать хранителем это устройство» is mine to press
 * on a device that does not keep the channel (wire §15).
 */
owner: boolean, 
/**
 * This device waits for the state of the keeper it takes over:
 * `handover` (from the device before) or `emergency` (recovering).
 */
taking?: "handover" | "emergency", 
/**
 * Since when it waits, unix seconds.
 */
taking_since?: number, 
/**
 * The keeper kept here belongs to a series the journal has gone past:
 * it hands its state over and acts no more («Передаём…»).
 */
handing: boolean, 
/**
 * This device holds the root of the channel: the keeper can be
 * recovered here if the device that keeps it is lost.
 */
has_root: boolean, 
/**
 * What a blob host said when it last refused a blob of the keeper kept
 * here, until every host takes them again: «Хранилище отказало: …».
 */
storage_error?: string, };
