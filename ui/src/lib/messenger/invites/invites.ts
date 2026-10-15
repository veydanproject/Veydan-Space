// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the screens of invitations read in the runtime's answers, kept apart
// so the rules are tested without a page. An invitation is one secret with
// a term, a number of uses and maybe one person it is for; a link, a code
// and a QR are its presets (internal plan, 1.6). The UI never takes a link
// apart: the runtime reads it (`links.inspect`) and says which person and
// which secret it carries. What the UI does read is what a person types:
// a code of six digits, said aloud and typed with spaces or a dash.

import type { InviteView, LinkView } from '../api';

/** A QR on the screen lives 15 minutes and is drawn anew while it is shown. */
export const QR_TTL_SECS = 15 * 60;
/** A new QR is drawn this long before the shown one ends: a camera aimed at the last second still reads a live one. */
export const QR_REFRESH_AHEAD_SECS = 20;
/** A code a person says aloud lives a day. */
export const CODE_TTL_SECS = 24 * 3600;

/** Six digits from what a person typed («482 913», «482-913»); `null` when it is not a code. */
export function normalizeCode(text: string): string | null {
  const digits = text.replace(/[\s -]/g, '');
  return /^\d{6}$/.test(digits) ? digits : null;
}

/** A code as it is read aloud: two groups of three. */
export function formatCode(code: string): string {
  return /^\d{6}$/.test(code) ? `${code.slice(0, 3)} ${code.slice(3)}` : code;
}

/**
 * What the address field of a new chat holds: a key (npub or hex), a link
 * of a person (the runtime reads it), or nothing it can start a chat with.
 */
export function addressKind(text: string): 'key' | 'link' | null {
  const q = text.trim();
  if (/^(?:nostr:)?(npub1[0-9a-z]{20,}|[0-9a-f]{64})$/i.test(q)) return 'key';
  if (/^veydan:\/\/contact\//i.test(q)) return 'link';
  return null;
}

/** A person's link as the runtime read it, with the secret of an invitation when it carries one. */
export interface InvitedContact {
  pubkey: string;
  name: string;
  /** The secret of the invitation; `null` for a plain link of a person. */
  invite: string | null;
  isMe: boolean;
  blocked: boolean;
}

/** The person and the secret of a link the runtime read; `null` for anything but a person's link. */
export function invitedContact(view: LinkView | undefined | null): InvitedContact | null {
  if (!view || view.kind !== 'contact') return null;
  const invite = (view as { invite?: unknown }).invite;
  return {
    pubkey: view.pubkey,
    name: view.name,
    invite: typeof invite === 'string' && invite ? invite : null,
    isMe: view.is_me,
    blocked: view.blocked,
  };
}

/** A QR without a term: shown until revoked. */
export const isPermanentQr = (v: InviteView): boolean => v.kind === 'qr' && v.expires_at == null;

/** Still lets someone in: not revoked, not past its term, uses left. */
export function isActive(v: InviteView, now: number): boolean {
  if (v.revoked) return false;
  if (v.expires_at != null && v.expires_at <= now) return false;
  return v.max_uses == null || v.used < v.max_uses;
}

/** Why an invitation no longer lets anyone in, for its line; `null` while it does. */
export function endedBy(v: InviteView, now: number): 'revoked' | 'expired' | 'used' | null {
  if (v.revoked) return 'revoked';
  if (v.max_uses != null && v.used >= v.max_uses) return 'used';
  if (v.expires_at != null && v.expires_at <= now) return 'expired';
  return null;
}

/**
 * The invitations a person manages in the settings, active first, newest
 * first within. A QR that lived its 15 minutes on the screen is left out:
 * the screen drew dozens of them, and each ended on its own.
 */
export function listed(list: readonly InviteView[], now: number): InviteView[] {
  return list
    .filter((v) => !(v.kind === 'qr' && !isPermanentQr(v) && !isActive(v, now)))
    .sort((a, b) => Number(isActive(b, now)) - Number(isActive(a, now)) || b.created_at - a.created_at);
}

/**
 * The QR to show again instead of drawing a new one: an active one of the
 * kind asked (for good, or timed with more than the refresh margin left),
 * the newest. Opening the screen twice in a minute draws one code, not two.
 */
export function reusableQr(list: readonly InviteView[], permanent: boolean, now: number): InviteView | null {
  const fits = list.filter((v) => {
    if (v.kind !== 'qr' || !isActive(v, now) || isPermanentQr(v) !== permanent) return false;
    return permanent || (v.expires_at ?? 0) - now > QR_REFRESH_AHEAD_SECS;
  });
  fits.sort((a, b) => b.created_at - a.created_at);
  return fits[0] ?? null;
}

/** `14:59`, `0:05`: what is left of a QR on the screen. */
export function countdown(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

/** Until when the invitations are paused; `null` when they are not (a pause in the past is over). */
export function pausedUntil(until: number | null | undefined, now: number): number | null {
  return until != null && until > now ? until : null;
}
