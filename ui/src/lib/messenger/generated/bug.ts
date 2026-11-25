// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Generated from Rust (messenger-runtime/src/bindings.rs). Do not edit:
// run `make msg-types` after changing the Rust types.

import type { NetDiagnosis } from './net';

/** A report of a bug to the project, as `messenger_bug_send` takes it: `text` 1 to 8000 characters once trimmed; `product` 1 to 32 of `a-z0-9-`; `version` at most 32, `os` 64, `locale` 16 characters; `modules` at most 16 lines of 200; `net` the diagnosis of `messenger_net_diagnose`, when attached (cut, then left out, past 64 KiB of the whole; the text never). Refusals: `bug_text`, `bug_field`, `payload_too_large`, `rate_limited` (more than three a minute), `no_host` (the user's own servers: nothing goes to the project), or the word of the server. */
export type BugInput = { 
/**
 * What happened, in the user's words: 1 to 8000 characters once
 * trimmed.
 */
text: string, 
/**
 * The product (`space`, `chat`…): 1 to 32 of `a-z`, `0-9`, `-`.
 */
product: string, 
/**
 * Its version, at most 32 characters.
 */
version: string, 
/**
 * The system and the machine (`android/aarch64`), at most 64.
 */
os: string, 
/**
 * The language of the app (`ru`), at most 16.
 */
locale: string, 
/**
 * The versions of the modules (`Camoufox: 135.0`): at most 16 lines
 * of at most 200 characters.
 */
modules: Array<string>, 
/**
 * The diagnosis of the network, as `messenger_net_diagnose` gave it,
 * when the user attaches it.
 */
net?: NetDiagnosis, };

/** `sent`: the server took it; `queued`: kept on this device and sent when the way is open again (the event `bug.report` tells of it then). */
export type BugStatus = "sent" | "queued";

/** What `messenger_bug_send` answers: `id` is the server's (12 hex) when it was sent now. */
export type BugResult = { status: BugStatus, 
/**
 * The server's id of the report (12 hex), when it was sent now.
 */
id: string | null, };
