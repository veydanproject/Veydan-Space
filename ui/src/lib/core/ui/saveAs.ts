// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// «Save as»: the file goes where the user picks. A computer and Android ask
// the save dialog for the place and the page's own command writes there. iOS
// has no place to write to outside the app (the address a dialog returns
// needs a security-scoped access), so the file is written into a staged copy
// in the app's cache and the system's document picker for exporting
// (`UIDocumentPickerViewController(forExporting:asCopy:)`) lets the user pick
// a folder in Files, where the system copies it itself. The two commands are
// the shell's, in every product (`files_stage`, `files_save`,
// crates/shell/src/commands/files.rs).

import { platform } from '$lib/core/platform';

export interface SaveAsOptions {
  /** The name the file is offered under. */
  defaultPath: string;
  filters?: { name: string; extensions: string[] }[];
}

/**
 * Asks where to save `options.defaultPath`, then has `write` write the file
 * to the path given (a path the app's fs scope lets be written). True when
 * the file was written and, on iOS, copied where the user picked; false when
 * the user gave up (`write` is not called on a computer and Android, and its
 * staged file is dropped on iOS).
 */
export async function saveAs(options: SaveAsOptions, write: (dest: string) => Promise<void>): Promise<boolean> {
  if (platform === 'ios') {
    const { invoke } = await import('@tauri-apps/api/core');
    const staged = await invoke<string>('files_stage', { name: options.defaultPath });
    try {
      await write(staged);
      return await invoke<boolean>('files_save', { path: staged });
    } catch (e) {
      // The staged copy may hold a secret (the recovery key): dropped at once.
      await invoke('files_save', { path: staged, discard: true }).catch(() => {});
      throw e;
    }
  }
  const { save } = await import('@tauri-apps/plugin-dialog');
  const dest = await save(options);
  if (!dest) return false;
  await write(dest);
  return true;
}
