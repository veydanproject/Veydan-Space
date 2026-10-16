// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Generated from Rust (messenger-runtime/src/bindings.rs). Do not edit:
// run `make msg-types` after changing the Rust types.

/** One request of a stranger: the chat, who, the one text that shows, when, and how many of their words wait behind it. */
export type RequestView = { 
/**
 * `dm:<hex>`
 */
chat_id: string, 
/**
 * Hex pubkey of the stranger.
 */
peer: string, 
/**
 * What the app calls them: a profile name, else a short npub.
 */
name: string, 
/**
 * The one visible text of the request (plain; empty when none shows).
 */
text: string, 
/**
 * Rumor time of that text (the peer's clock).
 */
at: number, 
/**
 * Held rows waiting behind it.
 */
held: number, 
/**
 * Where the queue places it: what `before` takes to page past it. Not
 * `at`: the queue is ordered by the activity of the chat, bounded by
 * my clock (`dm_relations::request_rows`), and the two part when the
 * text is dated ahead or is gone.
 */
cursor: number, };

/** A page of the queue of requests, newest first, as `messenger_moderation_requests` answers; `next_before` is what to pass for the next page, absent on the last. */
export type RequestsPage = { items: Array<RequestView>, 
/**
 * What to pass as `before` for the next page; absent on the last one.
 */
next_before?: number, 
/**
 * How many wait in all.
 */
total: number, };

/** What `messenger_moderation_request_act` does with a request: let the stranger in, say no, or shut the key. */
export type RequestAction = "accept" | "decline" | "block";

/** Whom I take messages from (`anyone` | `contacts`) and how many days a request waits (0: forever), as `messenger_moderation_dm_settings` answers. */
export type DmSettings = { 
/**
 * `anyone` | `contacts`
 */
dm_from: string, 
/**
 * `0` = forever.
 */
keep_days: number, };

/** A message waiting in the queue of a group, or hidden there. */
export type QueueItem = { message_id: string, group_id: string, 
/**
 * Hex pubkey of who wrote it.
 */
author: string, text: string, at: number, 
/**
 * Which version of the message the queue holds: an edit makes a new one.
 */
version: string, 
/**
 * How many more of the same author wait behind it.
 */
more_from_author: number, };

/** A page of the queue of a group, as `messenger_moderation_group_queue` answers. */
export type QueuePage = { items: Array<QueueItem>, next_before?: number, total: number, };

/** What of the queue of a group to show; a field left out means any. */
export type QueueFilter = { author?: string, since?: number, until?: number, text?: string, };

/** One act of moderation in a group, as `messenger_moderation_group_mod` takes it; tagged by `t`. */
export type ModAction = { "t": "hide", targets: Array<string>, } | { "t": "hide_author", author: string, from: number, to: number, } | { "t": "unhide", targets: Array<string>, } | { "t": "approve", target: string, version: string, } | { "t": "reject", target: string, } | { "t": "spam", author: string, target?: string, } | { "t": "mute", who: string, until?: number, } | { "t": "unmute", who: string, } | { "t": "trust", who: string, trusted: boolean, };

/** One line of the log of moderation of a group. */
export type LogEntry = { at: number, 
/**
 * Hex pubkey of the manager who acted.
 */
by: string, action: string, target: string, detail: string, };

/** A page of the log of moderation, as `messenger_moderation_group_log` answers. */
export type LogPage = { items: Array<LogEntry>, next_before?: number, };

/** When members may post: days 1 (Monday) to 7, `from` and `to` as HH:MM in `tz`. */
export type Schedule = { tz: string, days: Array<number>, from: string, to: string, };

/** The settings of moderation of a group: who may post (`all` | `managers`), when, and whose messages wait for a manager (`off` | `newcomers` | `all`). */
export type GroupModSettings = { 
/**
 * `all` | `managers`
 */
post_mode: string, schedule?: Schedule, 
/**
 * `off` | `newcomers` | `all`
 */
premod: string, };

/** What an invitation is to be, as `messenger_moderation_invite_create` takes it: a link, a code or a QR, for how many, for how long, for whom. */
export type InvitePreset = { 
/**
 * `link` | `code` | `qr`
 */
kind: string, max_uses?: number, ttl_secs?: number, 
/**
 * For one person only, hex.
 */
for_pubkey?: string, label: string, };

/** An invitation as `messenger_moderation_invite_list` lists them: `link` for a link and a QR (my contact link with the secret on it), none for a code; `used` counts the keys that came in by it. */
export type InviteView = { id: string, kind: string, secret: string, link?: string, label: string, created_at: number, expires_at?: number, max_uses?: number, used: number, for_pubkey?: string, revoked: boolean, };

/** The state of the invitations on this device, as `messenger_moderation_invite_status` answers: `paused_until` when somebody guessed too often (the event `dm.invite_paused` tells of it). */
export type InviteStatus = { paused_until?: number, };

/** Somebody I block, as `messenger_moderation_blocked` lists them: the key, its npub and what the app calls them. */
export type BlockedView = { 
/**
 * Hex pubkey.
 */
peer: string, npub: string, 
/**
 * What the app calls them: a nickname, a profile name, a short npub.
 */
name: string, 
/**
 * The network bans them (the manifest's `banned_keys`): the list shows
 * it and offers no unblock, which the runtime would refuse anyway.
 */
banned: boolean, };

/** Where the conduct reported happened; tagged by `t`. In a group, on a person, only the messages of the person reported and of the reporter are attached; on the group, those of any member. */
export type ReportContext = { "t": "dm" } | { "t": "group", group_id: string, } | { "t": "request" };

/** What a report is about. */
export type ReportReason = "child_harm" | "spam" | "harassment" | "illegal" | "other";

/** The kind of harm to a child, only with `child_harm`. */
export type ReportSubReason = "sexual" | "grooming" | "violence_illegal" | "suicide_selfharm" | "sect_recruitment";

/** Whom a report is on: `person` (the default) or `group`, the whole group of the context, whose owner the journal names as the accused. */
export type ReportTarget = "person" | "group";

/** A report of a person or of a whole group (`target`) to the moderators of the project, as `messenger_report_send` takes it: `attach` the last ten messages (ending with `message_id`, or the newest; on a group, of any member), `block` the person after it (not in a group). From a group the report carries its name and `message_id`; on a group `accused` is ignored (may be empty). Refusals: `rate_limited` (more than three a minute), `report_*` of a malformed input (`report_target`: a group reported outside of one), or the word of the server. */
export type ReportInput = { 
/**
 * The person reported: hex (an npub is taken too). Ignored for a
 * report on a group, whose owner the journal of the group names; the
 * page may pass an empty string then.
 */
accused: string, 
/**
 * A person (when absent) or the whole group of the context;
 * `report_target` when a group is reported outside of one.
 */
target?: ReportTarget | null, context: ReportContext, reason: ReportReason, 
/**
 * Only with `child_harm`.
 */
sub_reason?: ReportSubReason | null, 
/**
 * The reporter's own words, at most 2000 characters.
 */
comment?: string | null, 
/**
 * Attach the last messages (up to ten) with their proofs.
 */
attach: boolean, 
/**
 * The message the report starts from: the attached run ends with it;
 * with the newest when none.
 */
message_id?: string | null, 
/**
 * Block the person once the report is made (a chat of two or a
 * request; ignored for a group).
 */
block: boolean, };

/** `sent`: the server took it; `queued`: kept on this device and sent when the way is open again (the event `moderation.report` tells of it then). */
export type ReportStatus = "sent" | "queued";

/** What `messenger_report_send` answers: how many messages went and how many of them are proved to be their authors', and whether the person was blocked. */
export type ReportResult = { status: ReportStatus, 
/**
 * Messages attached, with a proof or without one.
 */
attached: number, 
/**
 * Of those, the ones whose author's key is proved (the check the
 * server makes).
 */
proved: number, 
/**
 * The person was blocked.
 */
blocked: boolean, };
