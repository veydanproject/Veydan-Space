// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Types of the ssh module's commands: connections, keys, SFTP.

export type SshAuthType = 'password' | 'key' | 'key_password';

export type SshStatus = 'connecting' | 'connected' | 'disconnected' | 'error';

export interface SshConnection {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  auth_type: SshAuthType;
  /** Secrets never leave the backend; these flags say whether one is stored. */
  has_password: boolean;
  has_private_key: boolean;
  has_key_passphrase: boolean;
  ssh_key_id: string | null;
  requires_2fa: boolean;
  totp_entry_id: string | null;
  proxy_id: string | null;
  /**
   * Name of the referenced proxy. `null` while `proxy_id` is set means the proxy
   * was deleted — the backend refuses to connect rather than going out directly,
   * so the UI must surface this instead of a plain "proxy" badge.
   */
  proxy_name: string | null;
  workspace_ids: string[];
  profile_ids: string[];
  connect_timeout_sec: number;
  keepalive_sec: number;
  terminal_theme: string | null;
  default_cols: number;
  default_rows: number;
  /** SHA256 host-key fingerprint pinned on first successful connect (TOFU). */
  server_fingerprint: string | null;
  last_connected_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface SshConnectionCreateInput {
  name: string;
  host: string;
  port?: number;
  username: string;
  auth_type: SshAuthType;
  password?: string | null;
  private_key?: string | null;
  key_passphrase?: string | null;
  ssh_key_id?: string | null;
  requires_2fa?: boolean;
  totp_entry_id?: string | null;
  proxy_id?: string | null;
  workspace_ids?: string[];
  profile_ids?: string[];
  connect_timeout_sec?: number;
  keepalive_sec?: number;
  terminal_theme?: string | null;
  default_cols?: number;
  default_rows?: number;
}

export interface SshConnectionUpdateInput {
  name?: string;
  host?: string;
  port?: number;
  username?: string;
  auth_type?: SshAuthType;
  password?: string | null;
  private_key?: string | null;
  key_passphrase?: string | null;
  ssh_key_id?: string | null;
  requires_2fa?: boolean;
  totp_entry_id?: string | null;
  proxy_id?: string | null;
  workspace_ids?: string[];
  profile_ids?: string[];
  connect_timeout_sec?: number;
  keepalive_sec?: number;
  terminal_theme?: string | null;
  default_cols?: number;
  default_rows?: number;
}

export interface SshSessionInfo {
  session_id: string;
  connection_id: string;
  connection_name: string;
  host: string;
  port: number;
  status: SshStatus;
  error: string | null;
  connected_at: string | null;
}

export interface SshKey {
  id: string;
  name: string;
  algorithm: string;
  bits: number | null;
  comment: string | null;
  public_key: string;
  /** Private material is fetched on demand via `api.sshKeys.exportPrivate`. */
  has_passphrase: boolean;
  fingerprint: string | null;
  source: 'generated' | 'imported';
  created_at: string;
  updated_at: string;
  usage_count: number;
}

export interface SshKeyImportInput {
  name: string;
  private_key: string;
  passphrase?: string | null;
}

export interface SshKeyGenerateInput {
  name: string;
  algorithm: 'ed25519' | 'rsa' | 'ecdsa';
  bits?: number | null;
  comment?: string | null;
  passphrase?: string | null;
}

export interface SshKeyUpdateInput {
  name?: string;
  comment?: string | null;
}

/** Unified directory entry — same shape for local FS and remote SFTP. */
export interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
  is_symlink: boolean;
  size: number;
  /** Epoch milliseconds. */
  mtime: number | null;
  mode: number;
  /** "rwxr-xr-x" */
  permissions: string;
  /** "0644" */
  octal: string;
  owner: string | null;
  group: string | null;
}

export interface SftpSessionInfo {
  connection_id: string;
  connection_name: string;
  home: string;
  connected_at: string;
}

export type TransferKind = 'download' | 'upload';

export interface TransferItemInput {
  src_path: string;
  dst_path: string;
  overwrite: boolean;
}

export interface TransferProgressEvent {
  transfer_id: string;
  kind: TransferKind;
  current_file: string;
  files_done: number;
  files_total: number;
  bytes_done: number;
  bytes_total: number;
}

export interface TransferDoneEvent {
  transfer_id: string;
  kind: TransferKind;
  error: string | null;
  cancelled: boolean;
  files_done: number;
  files_skipped: number;
}
