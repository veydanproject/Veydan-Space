// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Native file picking and large-attachment transfer progress shared by the
// desktop editor and the mobile note page.

import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { ATTACHMENT_TRANSFER_EVENT, type AttachmentTransfer } from '$lib/notes/api';

const IMAGE_EXTS = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'avif', 'svg'];

/** Native paths (desktop) or `content://` URIs (Android) of the picked files. */
export async function pickNativeFiles(imagesOnly = false): Promise<string[]> {
  const { open } = await import('@tauri-apps/plugin-dialog');
  const picked = await open({
    multiple: true,
    filters: imagesOnly ? [{ name: 'Images', extensions: IMAGE_EXTS }] : undefined,
  });
  if (!picked) return [];
  return Array.isArray(picked) ? picked : [picked];
}

export function transferKey(noteId: string, name: string): string {
  return `${noteId}/${name}`;
}

/** Live transfers keyed by `note_id/name`; finished entries without error are dropped. */
export function onAttachmentTransfer(
  cb: (transfers: Map<string, AttachmentTransfer>) => void,
): Promise<UnlistenFn> {
  const transfers = new Map<string, AttachmentTransfer>();
  return listen<AttachmentTransfer>(ATTACHMENT_TRANSFER_EVENT, (e) => {
    const t = e.payload;
    const key = transferKey(t.note_id, t.name);
    if (t.finished && !t.error) transfers.delete(key);
    else transfers.set(key, t);
    cb(new Map(transfers));
  });
}

export function transferPercent(t: AttachmentTransfer): number | null {
  return t.total ? Math.min(100, Math.round((t.done / t.total) * 100)) : null;
}
