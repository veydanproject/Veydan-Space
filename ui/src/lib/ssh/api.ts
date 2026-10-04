// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Commands of the ssh module: connections and keys, the terminal, SFTP and local files.

import { api as core, call, mockCommand } from '$lib/core/api';
import type {
  SshConnection,
  SshConnectionCreateInput,
  SshConnectionUpdateInput,
  SshSessionInfo,
  SshKey,
  SshKeyImportInput,
  SshKeyGenerateInput,
  SshKeyUpdateInput,
  FileEntry,
  SftpSessionInfo,
  TransferKind,
  TransferItemInput,
} from '$lib/ssh/types';

mockCommand('fs_home', '/home/dev');
mockCommand('fs_list', [
  { name: 'projects', path: '/home/dev/projects', is_dir: true, is_symlink: false, size: 4096, mtime: Date.now(), mode: 0o40755, permissions: 'rwxr-xr-x', octal: '0755', owner: '1000', group: '1000' },
  { name: 'readme.txt', path: '/home/dev/readme.txt', is_dir: false, is_symlink: false, size: 1234, mtime: Date.now(), mode: 0o100644, permissions: 'rw-r--r--', octal: '0644', owner: '1000', group: '1000' },
]);
mockCommand('sftp_session_list', []);

export const api = {
  ...core,
  ssh: {
    connectionList: (workspaceId?: string, profileId?: string, search?: string) =>
      call<SshConnection[]>('ssh_connection_list', { workspaceId, profileId, search }),
    connectionGet: (id: string) => call<SshConnection>('ssh_connection_get', { id }),
    connectionCreate: (input: SshConnectionCreateInput) =>
      call<SshConnection>('ssh_connection_create', { input }),
    connectionUpdate: (id: string, input: SshConnectionUpdateInput) =>
      call<SshConnection>('ssh_connection_update', { id, input }),
    connectionDelete: (id: string) => call<void>('ssh_connection_delete', { id }),
    connectionTrustFingerprint: (id: string, fingerprint: string) =>
      call<void>('ssh_connection_trust_fingerprint', { id, fingerprint }),
    connect: (connectionId: string) =>
      call<string>('ssh_connect', { connectionId }),
    disconnect: (sessionId: string) => call<void>('ssh_disconnect', { sessionId }),
    sendData: (sessionId: string, data: number[]) =>
      call<void>('ssh_send_data', { sessionId, data }),
    resize: (sessionId: string, cols: number, rows: number) =>
      call<void>('ssh_resize', { sessionId, cols, rows }),
    respondPrompt: (sessionId: string, response: string) =>
      call<void>('ssh_respond_prompt', { sessionId, response }),
    sessionList: () => call<SshSessionInfo[]>('ssh_session_list'),
    sessionRemove: (sessionId: string) => call<void>('ssh_session_remove', { sessionId }),
  },

  sshKeys: {
    list: () => call<SshKey[]>('ssh_key_list'),
    get: (id: string) => call<SshKey>('ssh_key_get', { id }),
    import: (input: SshKeyImportInput) => call<SshKey>('ssh_key_import', { input }),
    generate: (input: SshKeyGenerateInput) => call<SshKey>('ssh_key_generate', { input }),
    update: (id: string, input: SshKeyUpdateInput) =>
      call<SshKey>('ssh_key_update', { id, input }),
    delete: (id: string) => call<void>('ssh_key_delete', { id }),
    exportPrivate: (id: string) => call<string>('ssh_key_export_private', { id }),
  },

  sftp: {
    connect: (connectionId: string) =>
      call<SftpSessionInfo>('sftp_connect', { connectionId }),
    disconnect: (connectionId: string) =>
      call<void>('sftp_disconnect', { connectionId }),
    sessionList: () => call<SftpSessionInfo[]>('sftp_session_list'),
    home: (connectionId: string) => call<string>('sftp_home', { connectionId }),
    list: (connectionId: string, path: string) =>
      call<FileEntry[]>('sftp_list', { connectionId, path }),
    stat: (connectionId: string, path: string) =>
      call<FileEntry | null>('sftp_stat', { connectionId, path }),
    respondPrompt: (connectionId: string, response: string) =>
      call<void>('sftp_respond_prompt', { connectionId, response }),
    transferStart: (kind: TransferKind, connectionId: string, items: TransferItemInput[]) =>
      call<string>('sftp_transfer_start', { kind, connectionId, items }),
    transferCancel: (transferId: string) =>
      call<void>('sftp_transfer_cancel', { transferId }),
    mkdir: (connectionId: string, path: string) =>
      call<void>('sftp_mkdir', { connectionId, path }),
    createFile: (connectionId: string, path: string) =>
      call<void>('sftp_create_file', { connectionId, path }),
    rename: (connectionId: string, from: string, to: string) =>
      call<void>('sftp_rename', { connectionId, from, to }),
    delete: (connectionId: string, path: string) =>
      call<void>('sftp_delete', { connectionId, path }),
    chmod: (connectionId: string, path: string, mode: number) =>
      call<void>('sftp_chmod', { connectionId, path, mode }),
  },

  fs: {
    home: () => call<string>('fs_home'),
    list: (path: string) => call<FileEntry[]>('fs_list', { path }),
    stat: (path: string) => call<FileEntry | null>('fs_stat', { path }),
    mkdir: (path: string) => call<void>('fs_mkdir', { path }),
    createFile: (path: string) => call<void>('fs_create_file', { path }),
    rename: (from: string, to: string) => call<void>('fs_rename', { from, to }),
    delete: (path: string) => call<void>('fs_delete', { path }),
    chmod: (path: string, mode: number) => call<void>('fs_chmod', { path, mode }),
  },
};
