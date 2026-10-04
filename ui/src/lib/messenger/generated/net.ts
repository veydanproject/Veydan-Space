// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Generated from Rust (messenger-runtime/src/bindings.rs). Do not edit:
// run `make msg-types` after changing the Rust types.

/** Whether the project's servers are reached through a bridge: never, always, or when the direct way fails. */
export type NetMode = "off" | "on" | "auto";

/** A bridge the user added. */
export type BridgeView = { id: string, addr: string, };

/** The way to the project's servers as it is now. */
export type NetStatus = { mode: NetMode, 
/**
 * The project's servers are reached through a bridge right now.
 */
active: boolean, 
/**
 * The address of the bridge in use, while a connection to one is up.
 */
bridge: string | null, 
/**
 * Bridges known: added, listed, built in.
 */
bridges: number, private: Array<BridgeView>, 
/**
 * When the registry was last asked, unix seconds.
 */
list_checked_at: number | null, 
/**
 * A bridge would help and the user has not answered: show the offer.
 */
offer: boolean, 
/**
 * False with the user's own servers chosen: bridges carry only the
 * project's servers.
 */
available: boolean, };

/** What trying both ways found. */
export type Verdict = "direct" | "restricted" | "offline";

/** A check of the direct way and of a bridge. */
export type NetCheck = { direct: boolean, bridge: boolean, verdict: Verdict, };
