// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Generated from Rust (messenger-runtime/src/bindings.rs). Do not edit:
// run `make msg-types` after changing the Rust types.

/** A section of what a chat has shared: pictures and videos, files, links, voice and round videos. */
export type SharedSection = "visual" | "files" | "links" | "voice";

/** How many messages each section holds. Links are counted by message. */
export type SharedCounts = { visual: number, files: number, links: number, voice: number, };
