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
plan: string | null, until: number | null, } | { "t": "quota.90", pool: string | null, used: number | null, limit: number | null, } | { "t": "account.dormant", since: number | null, delete_after: number | null, } | { "t": "unknown" };
