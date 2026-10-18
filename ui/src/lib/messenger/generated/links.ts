// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Generated from Rust (messenger-runtime/src/bindings.rs). Do not edit:
// run `make msg-types` after changing the Rust types.

/** What I am to a group, as far as this device knows. */
export type GroupMembership = "joined" | "joining" | "requested" | "rejected" | "stale_link" | "left" | "removed" | "banned" | "disbanded";

/** Whether a group lets anyone in by its link. */
export type LinkGroupKind = "public" | "private";

/** A server as its card shows it, before it is added: what it has, whether it is Veydan's, whether it is mine already. */
export type ServerPreview = { 
/**
 * The card as this device writes it (`veydan://server/…`).
 */
link: string, 
/**
 * 16 hex: the id a peer address names (`<npub>@<id>`).
 */
id: string, 
/**
 * What the card calls it; may be empty.
 */
name: string, 
/**
 * `wss://…`
 */
relay: string, 
/**
 * The blob server, `https://…`.
 */
media: string | null, 
/**
 * The call node, `address:port#id`.
 */
call_node: string | null, 
/**
 * The push server of the server, `https://…`: where the device
 * registers for pushes through the gateway when the card is accepted.
 */
push_url: string | null, 
/**
 * `veydan`: phones are woken through the push of the Veydan network;
 * `off`: no push.
 */
push: string, 
/**
 * One of my servers already (of the home set or private): accepting
 * adds nothing.
 */
known: boolean, 
/**
 * Its relay is one of the Veydan network's (the manifest in use names
 * it): «серверы Veydan» rather than a server of somebody's own.
 */
is_veydan: boolean, };

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
membership: GroupMembership | null, 
/**
 * The card of the group's server the link carries (`srv=`): the
 * server is added first, after asking (`messenger_server_accept`).
 */
server: ServerPreview | null, 
/**
 * The group's relay is a server this device has (of the home set
 * or private): the link may be opened. Without it and without a
 * card, opening it is `server_unknown`.
 */
server_known: boolean, } | { "kind": "contact", link: string, pubkey: string, npub: string, 
/**
 * My name for them, then their own, then what the link says; may be empty.
 */
name: string, picture: string | null, nip05: string | null, is_me: boolean, is_contact: boolean, blocked: boolean, 
/**
 * The secret of their invitation, when the link carries one: a
 * request by it (`messenger_dm_action` with `invite`) is let in
 * without a request.
 */
invite: string | null, 
/**
 * The card of the server the person is talked to on (`srv=`):
 * the chat is the private one there (`messenger_server_accept`,
 * then `messenger_contact_open_private`), not the home one.
 */
server: ServerPreview | null, } | { "kind": "server" } & ServerPreview | { "kind": "vlink", link: string, id: string, 
/**
 * `address:port`.
 */
addr: string, 
/**
 * Already among the bridges added on this device.
 */
added: boolean, } | { "kind": "call_node", 
/**
 * The link as this device writes it, the token with it.
 */
link: string, id: string, 
/**
 * `address:port` of its control channel.
 */
addr: string, 
/**
 * The link carries an invitation (a link without one names the
 * node alone: its key is given by hand).
 */
has_token: boolean, 
/**
 * Already among the nodes of mine on this device: for a link with
 * an invitation, only when this device holds its own credentials
 * on the node (an entry of my own list leaves the invitation to
 * be exchanged).
 */
added: boolean, } | { "kind": "recover", link: string, 
/**
 * The account to recover, 64 hex.
 */
npub: string, 
/**
 * The check code as shown, `AB7K-2QZD-9MXP`.
 */
code: string, 
/**
 * My name for the person, when they are a contact; empty otherwise.
 */
name: string, } | { "kind": "unknown", link_type: string, } | { "kind": "invalid", code: string, };

/** A private server of mine as the settings list it; never its app key. */
export type PrivateServerView = { id: string, name: string | null, relay: string, media: string | null, call_node: string | null, 
/**
 * `veydan` | `off`
 */
push: string, 
/**
 * When it was added, unix seconds.
 */
added_at: number, 
/**
 * Its private chats on this device: what its removal takes with it.
 */
chats: number, 
/**
 * The groups that live on it (their relay is its relay).
 */
groups: number, };

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
