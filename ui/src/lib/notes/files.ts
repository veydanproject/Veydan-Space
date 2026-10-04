// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/** True when a paste carries OS file references the webview refuses to expose.
 *  WebKit (Linux/macOS) lists `text/uri-list` or `Files` in types but returns
 *  empty `files` and empty `getData()`; the paths must be read from the OS
 *  clipboard on the Rust side instead. */
export function pasteHasHiddenFiles(dt: DataTransfer | null): boolean {
  if (!dt || dt.files.length > 0) return false;
  const types = Array.from(dt.types);
  if (!types.includes('text/uri-list') && !types.includes('Files')) return false;
  return dt.getData('text/plain') === '' && dt.getData('text/html') === '';
}
