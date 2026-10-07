// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Generated from Rust (messenger-runtime/src/bindings.rs). Do not edit:
// run `make msg-types` after changing the Rust types.

/** A color of the bio's palette; the UI maps each to a token readable in both themes. */
export type Color = "red" | "orange" | "yellow" | "green" | "teal" | "blue" | "purple" | "pink" | "gray";

/** How a piece of a bio is written. */
export type Style = { bold: boolean, italic: boolean, strike: boolean, code: boolean, color: Color | null, };

/** A piece of a bio: text, a link (its text is its address) or a line break. */
export type Span = { "kind": "text", text: string, style: Style, } | { "kind": "link", url: string, text: string, style: Style, } | { "kind": "break" };

/** A link to a profile elsewhere as it is stored and sent: a platform id and a handle. */
export type SocialLink = { p: string, h: string, };

/** A checked link to a profile elsewhere, as the UI shows it; `url` may be empty. */
export type SocialView = { platform: string, 
/**
 * Name of the platform, e.g. "GitHub".
 */
name: string, 
/**
 * The handle as people write it there, e.g. "@name" or "u/name".
 */
handle: string, 
/**
 * Profile address; empty when the platform has none for this handle
 * (a Discord username).
 */
url: string, };

/** A platform the user can pick for a link. */
export type SocialPlatform = { id: string, name: string, 
/**
 * What to type, e.g. "@username" or "https://…".
 */
hint: string, };

/** A profile as the UI sees it. */
export type ProfileView = { pubkey: string, npub: string, name: string | null, display_name: string | null, 
/**
 * As other clients see it: the bio without its marks.
 */
about: string | null, picture: string | null, banner: string | null, website: string | null, nip05: string | null, lud16: string | null, nip05_verified: boolean, event_created_at: number, fetched_at: number, 
/**
 * The bio to show.
 */
bio: Array<Span>, 
/**
 * The bio to edit, always markup: ours when it agrees with `about`,
 * else `about` with its marks escaped, so that saving it unchanged
 * gives other clients the same `about` again.
 */
bio_source: string | null, socials: Array<SocialView>, };

/** What the user edits of their own profile; the avatar has commands of its own. */
export type ProfileInput = { name: string | null, display_name: string | null, 
/**
 * The bio, with its marks.
 */
about: string | null, website: string | null, nip05: string | null, lud16: string | null, 
/**
 * As typed: a platform and a handle, `@handle` or a profile address.
 */
socials: Array<SocialLink>, };

/** A contact card as the UI shows it. */
export type CardView = { pubkey: string, npub: string, 
/**
 * display_name → name → short npub.
 */
label: string, name: string | null, display_name: string | null, bio: Array<Span>, website: string | null, socials: Array<SocialView>, phone: string | null, 
/**
 * A `data:` URL of the picture.
 */
avatar: string | null, 
/**
 * The card is of me.
 */
is_me: boolean, 
/**
 * The person is in my contacts.
 */
is_contact: boolean, blocked: boolean, };

/** The part of a picked picture to keep, as fractions 0..1 of its preview. */
export type CropRect = { x: number, y: number, w: number, h: number, };

/** A picked picture, ready to be cropped. */
export type AvatarPreview = { 
/**
 * Names the picture for `avatar_set`; good for ten minutes, and only
 * until another picture is picked.
 */
token: string, 
/**
 * The whole picture, upright, as a `data:` url; the crop is given as
 * fractions of it.
 */
preview: string, 
/**
 * Of the picture itself, upright.
 */
width: number, height: number, };

/** My phone, which never goes into my public profile, and whether my card carries it by default. */
export type OwnPrivateView = { phone: string | null, 
/**
 * My card carries the phone unless the user unticks it.
 */
share_phone: boolean, };

/** The phone a contact sent me in its own card; `null` when none. */
export type ContactPrivateView = { pubkey: string, phone: string | null, };
