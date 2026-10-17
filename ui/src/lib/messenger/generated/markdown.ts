// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Generated from Rust (messenger-runtime/src/bindings.rs). Do not edit:
// run `make msg-types` after changing the Rust types.

/** How a piece of a line is written. */
export type MdStyle = { bold: boolean, italic: boolean, strike: boolean, };

/** A piece of a line: text, code, a link (its host shown beside its words; none for `veydan://`) or a line break. */
export type MdInline = { "kind": "text", text: string, style?: MdStyle, } | { "kind": "code", text: string, } | { "kind": "link", url: string, host: string | null, children: Array<MdInline>, } | { "kind": "break" };

/** A block of a Markdown message, as the runtime parsed it; rendered as elements and text nodes, never as HTML. */
export type MdBlock = { "kind": "para", inlines: Array<MdInline>, } | { "kind": "heading", level: number, inlines: Array<MdInline>, } | { "kind": "code", lang: string | null, text: string, } | { "kind": "quote", blocks: Array<MdBlock>, } | { "kind": "list", ordered: boolean, start: number, items: Array<MdItem>, } | { "kind": "table", head: Array<Array<MdInline>>, rows: Array<Array<Array<MdInline>>>, } | { "kind": "rule" };

/** An item of a list: done or not in a task list, and its blocks. */
export type MdItem = { 
/**
 * A task list: done or not.
 */
task: boolean | null, blocks: Array<MdBlock>, };
