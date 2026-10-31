// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// «Сохранить» of a QR code (ROO-297): the code the screen shows, as a PNG.
// A phone puts it into its gallery, album Pictures/Veydan
// (`messenger_gallery_save_picture`); a computer writes it where its save
// dialog says. The picture is drawn here from the SVG the screen shows, so
// what is saved is what is seen; the link itself never leaves the code.

import { get } from 'svelte/store';
import { t } from '$lib/core/i18n';
import { toast } from '$lib/core/ui/toast.svelte';
import { isTauriHost, messengerApi, messengerError } from '../api';

// The messenger may not ask $lib/core/platform; attach/library.ts asks the same way.
const android = typeof navigator !== 'undefined' && /android/i.test(navigator.userAgent);

/** The side of the saved picture: sharp in print and on a big screen, still small (a few dozen KB). */
const SIDE = 1024;

/** `<img src>` of a QR's SVG, as the screens show it: an image, nothing inside it can run. */
export function qrSrc(svg: string): string {
  return `data:image/svg+xml;base64,${btoa(unescape(encodeURIComponent(svg)))}`;
}

/** The SVG of a code drawn as a PNG of `side` pixels on white (the code has its quiet zone). */
export async function qrPng(svg: string, side = SIDE): Promise<Uint8Array> {
  const img = new Image();
  img.src = qrSrc(svg);
  await img.decode();
  const canvas = document.createElement('canvas');
  canvas.width = side;
  canvas.height = side;
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('no canvas');
  ctx.fillStyle = '#ffffff';
  ctx.fillRect(0, 0, side, side);
  // Modules stay squares: a camera reads edges.
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(img, 0, 0, side, side);
  const blob = await new Promise<Blob>((ok, fail) => canvas.toBlob((b) => (b ? ok(b) : fail(new Error('no png'))), 'image/png'));
  return new Uint8Array(await blob.arrayBuffer());
}

function base64(bytes: Uint8Array): string {
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

/** What a failed save says to the user. */
export function saveQrError(e: unknown, fallback: (e: unknown) => string): string {
  return messengerError(e).includes('storage_denied') ? (get(t) as (k: string) => string)('msg_qr_storage_denied') : fallback(e);
}

/**
 * The code saved as `name` (letters, digits, `-`, `_`; `.png` is added):
 * true when it was, false when the save dialog was closed. Says so with a
 * toast; throws what went wrong (`storage_denied` among it).
 */
export async function saveQr(svg: string, name: string): Promise<boolean> {
  const tr = get(t) as (k: string) => string;
  const png = await qrPng(svg);
  if (android) {
    await messengerApi.gallery.savePicture(base64(png), name);
    toast.success(tr('msg_qr_saved_gallery'));
    return true;
  }
  if (!isTauriHost) return false;
  const { save } = await import('@tauri-apps/plugin-dialog');
  const dest = await save({ defaultPath: `${name}.png`, filters: [{ name: 'PNG', extensions: ['png'] }] });
  if (!dest) return false;
  const { writeFile } = await import('@tauri-apps/plugin-fs');
  await writeFile(dest, png);
  toast.success(tr('msg_qr_saved'));
  return true;
}
