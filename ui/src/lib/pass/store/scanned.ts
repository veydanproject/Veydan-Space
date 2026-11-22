// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The otpauth link the phone's scanner read, on its way to the add form
// (design 16): in memory, not in the address, so the secret it holds stays
// out of the history. The form takes it once; nothing keeps it after that.

let pending = '';

/** The scanner leaves the link it read. */
export function putScanned(uri: string): void {
  pending = uri;
}

/** The add form takes the link, if there is one; it is gone after that. */
export function takeScanned(): string {
  const uri = pending;
  pending = '';
  return uri;
}
