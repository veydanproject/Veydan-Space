// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Commands of the browser module: workspaces, profiles, proxies, Camoufox, the clipper rules.

import { api as core, call, mockCommand } from '$lib/core/api';
import type {
  Workspace,
  CreateWorkspaceRequest,
  UpdateWorkspaceRequest,
  WorkspaceStats,
  WorkspaceColumn,
  CreateWorkspaceColumnRequest,
  UpdateWorkspaceColumnRequest,
  Profile,
  CreateProfileRequest,
  UpdateProfileRequest,
  Proxy,
  CreateProxyRequest,
  BulkProxyItem,
  BulkImportResult,
  ProxyCheckResult,
  PresetInfo,
  ProfileRawData,
  CamoufoxStatus,
  CaptureRule,
  ExportOptions,
} from '$lib/browser/types';
import type { SyncStatus } from '$lib/core/api';

// Answers in the plain browser (`vite dev` without Tauri).
mockCommand('fingerprint_presets', [
  { id: 'win10', label: 'Windows 10 / Chrome' },
  { id: 'win11', label: 'Windows 11 / Chrome' },
  { id: 'macos', label: 'macOS / Safari' },
  { id: 'linux', label: 'Linux / Firefox' },
]);
mockCommand('profiles_list', []);
mockCommand('proxies_list', []);
mockCommand('workspace_list', []);
mockCommand('profiles_list_by_workspace', []);
mockCommand('workspace_column_list', []);

export const api = {
  ...core,
  profiles: {
    list: () => call<Profile[]>('profiles_list'),
    get: (id: string) => call<Profile | null>('profile_get', { id }),
    create: (req: CreateProfileRequest) => call<Profile>('profile_create', { req }),
    update: (id: string, req: UpdateProfileRequest) =>
      call<Profile>('profile_update', { id, req }),
    delete: (id: string) => call<void>('profile_delete', { id }),
    clone: (id: string) => call<Profile>('profile_clone', { id }),
    launch: (id: string, force = false) => call<number>('profile_launch', { id, force }),
    stop: (id: string) => call<void>('profile_stop', { id }),
    isRunning: (id: string) => call<boolean>('profile_is_running', { id }),
    runningIds: () => call<string[]>('profiles_running_ids'),
    listByWorkspace: (workspaceId: string) =>
      call<Profile[]>('profiles_list_by_workspace', { workspaceId }),
    setTags: (id: string, tags: string[]) =>
      call<void>('profile_set_tags', { id, tags }),
    moveToKanbanColumn: (profileId: string, targetTag: string, kanbanOrder: number) =>
      call<void>('profile_move_to_kanban_column', { profileId, targetTag, kanbanOrder }),
    rawData: (id: string) => call<ProfileRawData>('profile_raw_data', { id }),
    exportJson: (id: string, options: ExportOptions) =>
      call<string>('profile_export_json', { id, options }),
    exportZip: (id: string, options: ExportOptions, outputPath: string) =>
      call<void>('profile_export_zip', { id, options, outputPath }),
    importJson: (json: string, workspaceId?: string) =>
      call<Profile>('profile_import_json', { json, workspaceId }),
    importZip: (filePath: string, workspaceId?: string) =>
      call<Profile>('profile_import_zip', { filePath, workspaceId }),
    importZipData: (dataB64: string, workspaceId?: string) =>
      call<Profile>('profile_import_zip_data', { dataB64, workspaceId }),
    importCookies: (id: string, cookiesJson: string) =>
      call<{ count: number; domains: string[] }>('profile_import_cookies', { id, cookiesJson }),
    exportCookies: (id: string) =>
      call<string>('profile_export_cookies', { id }),
    exportCookiesToFile: (id: string, outputPath: string) =>
      call<void>('profile_export_cookies_to_file', { id, outputPath }),
    exportJsonToFile: (id: string, options: ExportOptions, outputPath: string) =>
      call<void>('profile_export_json_to_file', { id, options, outputPath }),
  },

  proxies: {
    list: () => call<Proxy[]>('proxies_list'),
    get: (id: string) => call<Proxy | null>('proxy_get', { id }),
    create: (req: CreateProxyRequest) => call<Proxy>('proxy_create', { req }),
    bulkCreate: (items: BulkProxyItem[]) =>
      call<BulkImportResult>('proxies_bulk_create', { items }),
    update: (id: string, req: CreateProxyRequest) =>
      call<Proxy>('proxy_update', { id, req }),
    delete: (id: string) => call<void>('proxy_delete', { id }),
    /** Profiles and SSH connections still attached to this proxy. */
    usage: (id: string) =>
      call<{ profiles: string[]; ssh_connections: string[] }>('proxy_usage', { id }),
    check: (id: string) => call<ProxyCheckResult>('proxy_check', { id }),
    /** `type://user:pass@host:port` including the stored password */
    exportUrl: (id: string) => call<string>('proxy_export_url', { id }),
    trustFingerprint: (id: string, fingerprint: string, ip: string, country: string | null, city: string | null) =>
      call<void>('proxy_trust_fingerprint', { id, fingerprint, ip, country, city }),
  },

  workspaces: {
    list: () => call<Workspace[]>('workspace_list'),
    get: (id: string) => call<Workspace | null>('workspace_get', { id }),
    create: (req: CreateWorkspaceRequest) => call<Workspace>('workspace_create', { req }),
    update: (id: string, req: UpdateWorkspaceRequest) =>
      call<Workspace>('workspace_update', { id, req }),
    delete: (id: string, mode: 'move_to_default' | 'delete_all') =>
      call<void>('workspace_delete', { id, mode }),
    stats: (id: string) => call<WorkspaceStats>('workspace_stats', { id }),
    columns: {
      list: (workspaceId: string) =>
        call<WorkspaceColumn[]>('workspace_column_list', { workspaceId }),
      create: (workspaceId: string, req: CreateWorkspaceColumnRequest) =>
        call<WorkspaceColumn>('workspace_column_create', { workspaceId, req }),
      update: (id: string, req: UpdateWorkspaceColumnRequest) =>
        call<WorkspaceColumn>('workspace_column_update', { id, req }),
      delete: (id: string) => call<void>('workspace_column_delete', { id }),
    },
  },

  fingerprintPresets: () => call<PresetInfo[]>('fingerprint_presets'),

  camoufox: {
    status: () => call<CamoufoxStatus>('camoufox_status'),
    download: () => call<void>('camoufox_download'),
    downloadState: () =>
      call<{
        state: string;
        downloaded?: number;
        total?: number;
        percent?: number;
        version?: string;
        error?: string;
      }>('camoufox_download_state'),
    cancel: () => call<void>('camoufox_download_cancel'),
    latestVersion: () => call<string>('camoufox_latest_version'),
  },
  /** Rules of the web clipper: commands of Space's capture module, which writes into notes. */
  capture: {
    rulesGet: () => call<CaptureRule[]>('notes_capture_rules_get'),
    rulesSet: (rules: CaptureRule[]) => call<CaptureRule[]>('notes_capture_rules_set', { rules }),
  },
  sync: {
    ...core.sync,
    profileTakeRemote: (profileId: string) => call<SyncStatus>('sync_profile_files_take_remote', { profileId }),
    profilePushMine: (profileId: string) => call<SyncStatus>('sync_profile_files_push_mine', { profileId }),
  },
};

/** Device name from a `profile_launch` lease error, or null for other errors. */
export function leaseHolder(err: unknown): string | null {
  const msg = typeof err === 'string' ? err : (err as { message?: string })?.message ?? String(err);
  const i = msg.indexOf('profile_in_use:');
  return i >= 0 ? msg.slice(i + 'profile_in_use:'.length).trim() : null;
}
