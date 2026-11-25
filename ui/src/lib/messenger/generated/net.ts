// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Generated from Rust (messenger-runtime/src/bindings.rs). Do not edit:
// run `make msg-types` after changing the Rust types.

import type { CallNodeClass } from './calls';

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

/** Which servers the user chose: the project's (the diagnosis is made), their own or none yet (nothing is asked, the lists are empty). */
export type DiagnosisServers = "veydan" | "own" | "unchosen";

/** What a host of the project is to the manifest: a relay, a media server, a push server, a registry of bridges, a source of the manifest. */
export type HostKind = "relay" | "media" | "push" | "registry" | "source";

/** One host of the project asked the direct way, never through a bridge: any HTTP answer is `ok`; `error` is one word: `timeout`, `dns`, `tls`, `refused`, `reset` or `other`. */
export type DirectCheck = { 
/**
 * The host name alone, as the manifest names it.
 */
host: string, kind: HostKind, 
/**
 * Any HTTP answer came.
 */
ok: boolean, 
/**
 * How long the answer took, milliseconds.
 */
ms: number | null, 
/**
 * Why nothing came: `timeout`, `dns`, `tls`, `refused`, `reset` or
 * `other`.
 */
error: string | null, };

/** The push of `bytes` (256 KB) to a registry, the direct way: past the size at which a throttled way stalls. */
export type CanaryCheck = { 
/**
 * A registry took the bytes and answered with success.
 */
ok: boolean, ms: number | null, 
/**
 * What the test pushes.
 */
bytes: number, };

/** One bridge tried on its own (a client that knows it alone): a connection and the probe of `bytes` each way; `error` as in `DirectCheck`. */
export type BridgeCheck = { 
/**
 * The address of the bridge (`address:port`) when a public list
 * names it, [`PRIVATE_BRIDGE`] for one the user added; never its id.
 */
addr: string, ok: boolean, ms: number | null, 
/**
 * What the probe moves each way.
 */
bytes: number, 
/**
 * Why it failed, as [`DirectCheck::error`].
 */
error: string | null, };

/** One call node of the project or of a volunteer that a call may use now: HELLO and one STUN binding, each timed on its own. */
export type CallNodeCheck = { 
/**
 * The address of the node (`address:port`), from the manifest or the
 * registry's list.
 */
addr: string, 
/**
 * The region the registry names for it, when it does.
 */
region: string | null, class: CallNodeClass, hello_ok: boolean, hello_ms: number | null, stun_ok: boolean, stun_ms: number | null, };

/** The diagnosis of the network, as `messenger_net_diagnose` answers (at most 45 s; what did not finish is `ok: false`, `error: "timeout"`). Only the hosts of the project, the addresses of bridges and call nodes and timings: no key, no address of this device, nothing of the user's own servers or chats. With the user's own servers, or none chosen, nothing is asked and the lists are empty. */
export type NetDiagnosis = { v: number, 
/**
 * When it was made, unix seconds.
 */
at: number, servers_mode: DiagnosisServers, mode: NetMode, 
/**
 * The project's servers are reached through a bridge now.
 */
active: boolean, 
/**
 * The address of the bridge in use, while a connection to one is up.
 */
bridge_in_use: string | null, direct: Array<DirectCheck>, 
/**
 * None when the manifest names no registry.
 */
canary: CanaryCheck | null, bridges: Array<BridgeCheck>, calls: Array<CallNodeCheck>, };
