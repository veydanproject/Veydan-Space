// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Generated from Rust (messenger-runtime/src/bindings.rs). Do not edit:
// run `make msg-types` after changing the Rust types.

/** What I am to a group, as far as this device knows. */
export type GroupMembership = "joined" | "joining" | "requested" | "rejected" | "stale_link" | "left" | "removed" | "banned" | "disbanded";

/** Whether a group lets anyone in by its link. */
export type LinkGroupKind = "public" | "private";

/** What a link inside leads to, as the runtime sees it. The UI never takes a link apart itself. */
export type LinkView = { "kind": "group", 
/**
 * The link as this device writes it; cards act on this one.
 */
link: string, group_id: string, group_kind: LinkGroupKind, name: string, relay: string, owner: string, 
/**
 * Known only for a group this device has met.
 */
picture: string | null, members: number | null, 
/**
 * Empty when this device has never met the group.
 */
membership: GroupMembership | null, } | { "kind": "contact", link: string, pubkey: string, npub: string, 
/**
 * My name for them, then their own, then what the link says; may be empty.
 */
name: string, picture: string | null, nip05: string | null, is_me: boolean, is_contact: boolean, blocked: boolean, } | { "kind": "vlink", link: string, id: string, 
/**
 * `address:port`.
 */
addr: string, 
/**
 * Already among the bridges added on this device.
 */
added: boolean, } | { "kind": "unknown", link_type: string, } | { "kind": "invalid", code: string, };

/** What a page outside says about itself. */
export type LinkPreview = { 
/**
 * The address that was asked.
 */
url: string, 
/**
 * Host that answered, after redirects.
 */
host: string, title: string | null, description: string | null, site_name: string | null, 
/**
 * `data:` with the picture itself: showing it asks nobody.
 */
image: string | null, };
