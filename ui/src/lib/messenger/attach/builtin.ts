// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the messenger itself gives to the sheet of the paperclip: the
// gallery, a file, the picture on the clipboard. They are given like any
// module's item, through `provideAttachAction`, and taken back with it.

import { messengerApi } from '../api';
import { provideAttachAction, type AttachResult } from '../seams/attach.svelte';
import { clipboardHasPicture, clipboardPicture } from '../media/clipboard';
import GalleryPanel from './GalleryPanel.svelte';
import { mediaLibrary } from './library';

const files = (paths: string[]): AttachResult | null => (paths.length ? { type: 'files', paths, original: false } : null);

/** Gives the messenger's items; returns the way to take them back. */
export function provideMessengerAttach(): () => void {
  const stops = [
    provideAttachAction({
      id: 'gallery', order: 10, icon: 'image', label: 'msg_attach_gallery', produces: 'files',
      panel: GalleryPanel,
    }),
    provideAttachAction({
      id: 'file', order: 20, icon: 'file', label: 'msg_attach_file', produces: 'files',
      // A file goes as it is: a HEIC stays a HEIC (a photo of the gallery is made a JPEG).
      run: async () => {
        const paths = await mediaLibrary().pickSystem({ multiple: true, kind: 'any' });
        return paths.length ? { type: 'files', paths, original: false, asFile: true } : null;
      },
    }),
    provideAttachAction({
      id: 'clipboard', order: 30, icon: 'copy', label: 'msg_attach_clipboard', produces: 'files',
      // Chats only, and only while the phone's clipboard holds a picture (read on the tap only: Android 12+ shows a toast).
      available: (ctx) => (ctx.surface === 'dm' || ctx.surface === 'group') && clipboardHasPicture(),
      run: async () => {
        // A computer's clipboard is read by the runtime; a phone's (the runtime answers nothing there) through the shell.
        let paths = await messengerApi.media.pasteClipboard();
        if (!paths.length) paths = [clipboardPicture()].filter((u): u is string => !!u);
        return files(paths);
      },
    }),
  ];
  return () => stops.forEach((stop) => stop());
}
