// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Commands of the pass module: passwords, TOTP, the generator history.

import { api as core, call, mockCommand } from '$lib/core/api';
import type {
  TotpEntry,
  TotpCode,
  TotpPreview,
  TotpAddRequest,
  TotpUpdateRequest,
  PasswordEntry,
  PasswordCreateRequest,
  PasswordUpdateRequest,
  RevealedSecret,
} from '$lib/pass/types';


export const api = {
  ...core,
  pwgen: {
    list: () => call<{ id: string; password: string; created_at: string }[]>('pwgen_history_list'),
    add: (password: string) =>
      call<{ id: string; password: string; created_at: string }>('pwgen_history_add', { password }),
    clear: () => call<void>('pwgen_history_clear'),
    trim: (limit: number) => call<void>('pwgen_history_trim', { limit }),
  },

  /** The lock of the app and the vault key; the shell owns these commands. */

  passwords: {
    list: () => call<PasswordEntry[]>('password_list'),
    get: (id: string) => call<PasswordEntry>('password_get', { id }),
    create: (req: PasswordCreateRequest) => call<PasswordEntry>('password_create', { req }),
    update: (id: string, req: PasswordUpdateRequest) => call<PasswordEntry>('password_update', { id, req }),
    delete: (id: string) => call<void>('password_delete', { id }),
    reveal: (id: string, field: 'password' | 'note') => call<RevealedSecret>('password_reveal', { id, field }),
    copy: (id: string) => call<void>('password_copy', { id }),
    vaultReset: (password: string) => call<void>('password_vault_reset', { password }),
  },

  totp: {
    list: () => call<TotpEntry[]>('totp_list'),
    add: (req: TotpAddRequest) => call<TotpEntry>('totp_add', { req }),
    update: (id: string, req: TotpUpdateRequest) => call<TotpEntry>('totp_update', { id, req }),
    delete: (id: string) => call<void>('totp_delete', { id }),
    generateCode: (id: string) => call<TotpCode>('totp_generate_code', { id }),
    generateCodes: (ids: string[]) => call<TotpCode[]>('totp_generate_codes', { ids }),
    previewUri: (uri: string) => call<TotpPreview>('totp_preview_uri', { uri }),
  },
};
