// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// «Поделиться» outside Veydan (ROO-173): a text with a link goes into
// another app the user picks there, to the addressee they pick there. The
// sheet of it is ShareOutSheet.svelte; sending inside Veydan is ShareDialog.

import { get } from 'svelte/store';
import { t } from '$lib/core/i18n';
import { copied as copiedToast } from '$lib/core/ui/toast.svelte';
import { messengerApi } from '../api';

// The messenger may not ask $lib/core/platform; attach/library.ts asks the same way.
const android = typeof navigator !== 'undefined' && /android/i.test(navigator.userAgent);

/** What is shared: the words with the link in them, a letter's subject, the link alone. */
export interface ShareOutItem {
  text: string;
  subject: string;
  url: string;
}

/**
 * The ways out: `send` the phone's list of apps, `sms` its SMS app with no
 * addressee, `mail` a letter, `copy` the link to the clipboard, `qr` the
 * link drawn on the screen (the sheet itself).
 */
export type ShareOutWay = 'send' | 'sms' | 'mail' | 'copy' | 'qr';

/** The ways this platform has, in the order of the sheet. A computer has no list of apps and no SMS. */
export function shareOutWays(on: string = android ? 'android' : 'other'): ShareOutWay[] {
  return on === 'android' ? ['send', 'sms', 'mail', 'copy', 'qr'] : ['mail', 'copy', 'qr'];
}

/** The item through one way; `qr` is the sheet's to draw. Throws what the platform answered. */
export async function shareOut(item: ShareOutItem, way: Exclude<ShareOutWay, 'qr'>): Promise<void> {
  if (way === 'copy') {
    await navigator.clipboard.writeText(item.url);
    copiedToast();
    return;
  }
  await messengerApi.shareOut(way, item.text, way === 'mail' ? item.subject : undefined);
}

/**
 * My contact as words for another app: one sentence with the link, nothing
 * else of me (no phone, no name). The words are changed here only.
 */
export function contactShareItem(url: string): ShareOutItem {
  const tr = get(t) as (k: string, p?: Record<string, string>) => string;
  return { text: tr('msg_share_out_contact_text', { link: url }), subject: tr('msg_share_out_contact_subject'), url };
}

/** A group's link as words for another app: one sentence with the link, not even the group's name. */
export function groupShareItem(url: string): ShareOutItem {
  const tr = get(t) as (k: string, p?: Record<string, string>) => string;
  return { text: tr('msg_share_out_group_text', { link: url }), subject: tr('msg_share_out_group_subject'), url };
}

/** A channel's link as words for another app, as groupShareItem. */
export function channelShareItem(url: string): ShareOutItem {
  const tr = get(t) as (k: string, p?: Record<string, string>) => string;
  return { text: tr('msg_share_out_channel_text', { link: url }), subject: tr('msg_share_out_channel_subject'), url };
}
