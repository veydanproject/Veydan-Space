// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Generated from Rust (messenger-runtime/src/bindings.rs). Do not edit:
// run `make msg-types` after changing the Rust types.

/** A section of what a chat has shared: pictures and videos, files, links, voice and round videos. */
export type SharedSection = "visual" | "files" | "links" | "voice";

/** How many messages each section holds. Links are counted by message. */
export type SharedCounts = { visual: number, files: number, links: number, voice: number, };

/** What «load earlier» of a direct chat came to: messages older than the one asked before are there to read (`older`), or the server has nothing older (`end`). */
export type DmHistoryPage = { 
/**
 * Messages of the chat older than `before` there are on this device
 * now (0: nothing older came). The page reads them as usual.
 */
older: number, 
/**
 * The relays of the server have nothing older than what this device
 * holds: asking again brings nothing (until another set of relays).
 */
end: boolean, };
