// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Types of the pass module's commands: TOTP and passwords.

export interface TotpEntry {
  id: string;
  name: string;
  issuer: string | null;
  algorithm: string;
  digits: number;
  period: number;
  tags: string[];
  created_at: string;
  updated_at: string;
  last_used_at: string | null;
}

export interface TotpCode {
  id: string;
  code: string;
  seconds_left: number;
}

export interface TotpPreview {
  name: string;
  issuer: string | null;
  secret_masked: string;
  algorithm: string;
  digits: number;
  period: number;
}

export interface TotpAddRequest {
  name: string;
  issuer?: string | null;
  secret?: string | null;
  uri?: string | null;
  algorithm?: string;
  digits?: number;
  period?: number;
  tags: string[];
}

export interface TotpUpdateRequest {
  name?: string;
  issuer?: string | null;
  tags?: string[];
}

export interface PasswordEntry {
  id: string;
  title: string;
  username: string | null;
  url: string | null;
  has_note: boolean;
  totp_ids: string[];
  tags: string[];
  created_at: string;
  updated_at: string;
}

export interface PasswordCreateRequest {
  title: string;
  username?: string | null;
  url?: string | null;
  password: string;
  note?: string | null;
  totp_ids?: string[];
  tags?: string[];
}

export interface PasswordUpdateRequest {
  title?: string;
  username?: string | null;
  url?: string | null;
  password?: string | null;
  note?: string | null;
  clear_note?: boolean;
  totp_ids?: string[];
  tags?: string[];
}

export interface RevealedSecret {
  value: string;
}
