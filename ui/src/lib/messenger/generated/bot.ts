// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Generated from Rust (messenger-runtime/src/bindings.rs). Do not edit:
// run `make msg-types` after changing the Rust types.

/** A card of the bot of Veydan Cloud: the `bot_card` of a message whose `content_type` is `bot_card`, set only on what the pinned key sealed. The words of a card are the app's own, by `t`; the values are numbers (unix seconds, bytes) or short words without a space, a dot or a slash, `null` when absent or not what they should be. `unknown`: a type this app does not know, drawn without text. */
export type BotCard = { "t": "recovery.requested", 
/**
 * What «Это не я — отменить» cancels.
 */
request_id: string | null, created_at: number | null, expires_at: number | null, } | { "t": "recovery.escrow_read", at: number | null, } | { "t": "recovery.restored", at: number | null, } | { "t": "account.deleted", receipt: string | null, at: number | null, } | { "t": "plan.expiring", 
/**
 * The id of the plan (`plus`), not a name to show as it is.
 */
plan: string | null, until: number | null, } | { "t": "quota.90", pool: string | null, used: number | null, limit: number | null, } | { "t": "account.dormant", since: number | null, delete_after: number | null, } | { "t": "account.downgraded", 
/**
 * The id of the plan that ended.
 */
plan: string | null, 
/**
 * The limits of free, bytes.
 */
files_bytes: number | null, text_bytes: number | null, } | { "t": "web.login", 
/**
 * The sign-in, 16 hex: what «Это не я» denies.
 */
id: string | null, 
/**
 * Exactly 6 digits, as text (leading zeros kept); without it the
 * card shows no code.
 */
code: string | null, 
/**
 * Two capital letters (ISO 3166-1 alpha-2) of where the request came from.
 */
country: string | null, 
/**
 * The family of the browser: `chrome`, `firefox`, `safari`, `edge`, `opera`, `other`.
 */
ua_family: string | null, expires_at: number | null, } | { "t": "web.signed_in", 
/**
 * The session, 16 hex.
 */
session: string | null, country: string | null, ua_family: string | null, at: number | null, } | { "t": "web.confirm", 
/**
 * The action waiting, 16 hex.
 */
id: string | null, 
/**
 * `delete`: the deletion of the account.
 */
action: string | null, country: string | null, ua_family: string | null, expires_at: number | null, } | { "t": "order.paid", 
/**
 * The order, 16 hex.
 */
order_id: string | null, 
/**
 * The id of the plan.
 */
plan: string | null, period_days: number | null, until: number | null, 
/**
 * 3 to 5 capital letters (`USD`, `USDT`).
 */
currency: string | null, amount_minor: number | null, at: number | null, } | { "t": "order.key", order_id: string | null, 
/**
 * 5 groups of 4 Crockford characters joined by `-`.
 */
key: string | null, plan: string | null, period_days: number | null, currency: string | null, amount_minor: number | null, at: number | null, } | { "t": "unknown" };
