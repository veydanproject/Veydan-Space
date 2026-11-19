// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What a paste into a chat carries, and the phone's clipboard through the
// shell's bridge. Text wins: what Excel or Word copies holds a picture of
// the text too, and the text is what is meant; a picture or a file is
// attached when there is no text.
//
// Where a paste carries files, the webview hands them over (WebView2,
// Chromium on a phone, often WKWebView). Where it only names them
// (WebKitGTK and WKWebView list `Files`, `text/uri-list` or `image/*` and
// give nothing), the computer's clipboard is read by the runtime
// (`messenger_clipboard_paste`), the phone's through `VeydanChrome`
// ($lib/core/ui/chrome: Android's object, the iPhone's commands).

import { chromeBridge } from '$lib/core/ui/chrome';

export type PasteTake =
  | { kind: 'text' }
  | { kind: 'files'; files: File[] }
  /** Something to attach the webview does not hand over: read the clipboard itself. */
  | { kind: 'native' }
  | { kind: 'none' };

export function classifyPaste(dt: DataTransfer | null): PasteTake {
  if (!dt) return { kind: 'none' };
  // Files copied in a file manager (Finder, Nautilus) come with their names or paths as text too: they are files.
  const copiedFiles = /^file:\/\//m.test(dt.getData('text/uri-list'));
  if (!copiedFiles && dt.getData('text/plain').trim()) return { kind: 'text' };
  const files = Array.from(dt.files ?? []);
  if (!files.length) {
    for (const item of Array.from(dt.items ?? [])) {
      const f = item.kind === 'file' ? item.getAsFile() : null;
      if (f) files.push(f);
    }
  }
  if (files.length) return { kind: 'files', files };
  const types = Array.from(dt.types ?? []);
  if (types.some((t) => t === 'Files' || t === 'text/uri-list' || t.startsWith('image/'))) return { kind: 'native' };
  return { kind: 'none' };
}

const bridge = chromeBridge;

/** Whether the phone's clipboard holds a picture (only its description is read: Android says nothing). */
export function clipboardHasPicture(): boolean {
  try { return !!bridge()?.clipboardHasImage?.(); } catch { return false; }
}

/**
 * The picture on the phone's clipboard: a `content://` URI on Android, a copy in the app's cache on
 * iOS. Read on the user's tap only (Android 12+ shows a toast, iOS asks «Allow Paste»).
 */
export async function clipboardPicture(): Promise<string | null> {
  try { return (await bridge()?.clipboardImage?.()) ?? null; } catch { return null; }
}

/** Whether pictures a keyboard inserts come to the page as `veydan-received-content` (a phone). */
export function takePictures(takes: boolean) {
  try { bridge()?.setTakesPictures?.(takes); } catch { /* not a phone */ }
}

/** The event the phone's shell sends with the `content://` URIs of pictures a keyboard inserted. */
export const RECEIVED_CONTENT = 'veydan-received-content';
