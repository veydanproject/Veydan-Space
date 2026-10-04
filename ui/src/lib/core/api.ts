// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The call wrapper and the commands of the shell (lock, sync, settings, system,
// demo, start, the switches of the modules). A module's commands live in its own `api.ts`, which
// spreads this object and adds its groups (platform-spec 11.5), so call sites
// read `api.<group>.<command>` wherever they are.

import type { LockStatus, LockSecret, LockSetResult, HostInfo, UpdateCheck } from '$lib/core/types';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** Label of the current Tauri window ('main' in the browser/dev mock). */
export function windowLabel(): string {
  if (typeof window === 'undefined') return 'main';
  const internals = (
    window as unknown as {
      __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string } } };
    }
  ).__TAURI_INTERNALS__;
  return internals?.metadata?.currentWindow?.label ?? 'main';
}

/** True in a window of a module's own (`ModuleDef.windows`): theme + chrome only. */
export function isSecondaryWindow(): boolean {
  return windowLabel() !== 'main';
}

/** A reference to an entity of any module: the two halves of a tag `kind:id`. */
export interface LabelRef {
  kind: string;
  id: string;
}

/**
 * The answer of `labels_resolve` for one reference; `name` is null while the
 * catalog of names has none. `color` is the label's color (a workspace's)
 * when it is a hex color `#rgb` / `#rrggbb`, else null.
 */
export interface LabelName extends LabelRef {
  name: string | null;
  color: string | null;
}

/**
 * The names the plain browser's `labels_resolve` knows: the entities of
 * Space a demo entry of another module links to. Any other reference has no
 * name and is shown by its kind and short id.
 */
const devLabels: Record<string, { name: string; color?: string }> = {
  'workspace:demo-ws-smm': { name: 'SMM', color: '#22c55e' },
  'profile:demo-pr-brand-a': { name: 'Brand A' },
  'note:demo-note-deploy': { name: 'Deploy' },
};

/**
 * Answers of `call` in a plain browser (`vite dev` without Tauri); modules
 * add theirs. A function is called with the arguments of the command.
 */
const devMocks: Record<string, unknown> = {
  update_supported: true,
  app_start_error: null,
  labels_resolve: (args?: Record<string, unknown>): LabelName[] =>
    ((args?.items ?? []) as LabelRef[]).map(({ kind, id }) => {
      const label = devLabels[`${kind}:${id}`];
      return { kind, id, name: label?.name ?? null, color: label?.color ?? null };
    }),
};

/** A module's answer for a command in the plain browser. */
export function mockCommand(cmd: string, value: unknown): void {
  devMocks[cmd] = value;
}

export async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauri) {
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<T>(cmd, args);
  }
  console.warn(`[dev-browser] invoke('${cmd}')`, args ?? '');
  const mock = cmd in devMocks ? devMocks[cmd] : [];
  return (typeof mock === 'function' ? mock(args) : mock) as T;
}

export const api = {
  /** The lock of the app and the vault key; the shell owns these commands. */
  lock: {
    status: () => call<LockStatus>('lock_status'),
    set: (secret: LockSecret | null, current?: string) =>
      call<LockSetResult>('lock_set', {
        password: secret?.password ?? null,
        kind: secret?.kind ?? null,
        hint: secret?.hint ?? null,
        current: current ?? null,
      }),
    recoveryRegenerate: (current: string) => call<string>('lock_recovery_regenerate', { current }),
    recoveryCheck: (code: string) => call<void>('lock_recovery_check', { code }),
    recover: (code: string, secret: LockSecret) =>
      call<LockSetResult>('lock_recover', { code, ...secret }),
    timeoutSet: (minutes: number) => call<LockStatus>('lock_timeout_set', { minutes }),
    unlock: (password: string) => call<LockStatus>('lock_unlock', { password }),
    lock: () => call<LockStatus>('lock_lock'),
    touch: () => call<void>('lock_touch'),
  },

  media: {
    /** Lets the webview answer camera / microphone requests before `getUserMedia`. */
    grantAccess: () => call<void>('media_grant_access'),
  },

  system: {
    openUrl: (url: string) => call<void>('open_url', { url }),
    /** OS clipboard via the backend; works outside a user-gesture context */
    clipboardWriteText: (text: string) => call<void>('clipboard_write_text', { text }),
    updateSupported: () => call<boolean>('update_supported'),
    hostInfo: () => call<HostInfo>('host_info'),
  },

  /** The catalog of names (platform-spec 10.2): what an entity of any module is called. */
  labels: {
    /**
     * One answer per reference, in their order: the owner's name where the
     * product has the module, the synced label where it has not, null where
     * there is none. Names only; nothing is written.
     */
    resolve: (items: LabelRef[]) => call<LabelName[]>('labels_resolve', { items }),
  },

  /** The check for a new version where there is no updater (a phone). */
  update: {
    /** `manual: false` is the check of the start: once a day, not on a metered network, silent. */
    check: (manual: boolean) => call<UpdateCheck>('update_check', { manual }),
    /** Opens the release page of the version the last check found. */
    open: () => call<void>('update_open'),
  },

  sync: {
    getConfig: () => call<SyncConfig>('sync_get_config'),
    setConfig: (cfg: SyncConfig) => call<void>('sync_set_config', { cfg }),
    probe: () => call<'empty' | 'vault' | 'foreign'>('sync_probe'),
    createVault: (passphrase: string) => call<SyncStatus>('sync_create_vault', { passphrase }),
    joinVault: (passphrase: string) => call<SyncStatus>('sync_join_vault', { passphrase }),
    leave: () => call<SyncStatus>('sync_leave'),
    changePassphrase: (old: string, newPassphrase: string) =>
      call<void>('sync_change_passphrase', { old, new: newPassphrase }),
    status: () => call<SyncStatus>('sync_status'),
    runNow: () => call<SyncStatus>('sync_run_now'),
    /** Background cycle if a vault is joined and the rate limit allows. */
    trigger: () => call<void>('sync_trigger'),
    debugGet: () => call<SyncDebugState>('sync_debug_get'),
    debugSet: (enabled: boolean) => call<void>('sync_debug_set', { enabled }),
    debugClear: () => call<void>('sync_debug_clear'),
  },

  settings: {
    getTray: () => call<TraySettings>('tray_settings_get'),
    setTray: (s: TraySettings) =>
      call<void>('tray_settings_set', {
        minimizeToTray: s.minimize_to_tray,
        closeToTray: s.close_to_tray,
        startHidden: s.start_hidden,
      }),
    setTrayLabels: (labels: TrayLabels) => call<void>('tray_set_labels', { labels }),
    setLocale: (locale: string) => call<void>('app_locale_set', { locale }),
    getLocale: () => call<string>('app_locale_get'),
    windowMinimize: () => call<void>('window_minimize'),
  },

  demo: {
    seed: (locale: 'ru' | 'en') => call<void>('demo_seed', { locale }),
  },

  app: {
    /** Non-null in a degraded start: no other command works until the cause is fixed. */
    startError: () => call<StartError | null>('app_start_error'),
    clearData: () => call<void>('app_clear_data'),
  },

  /** Which modules this device keeps on (platform-spec 12); the shell owns these. */
  modules: {
    list: () => call<ModulesView>('modules_list'),
    /** Refused with `modules_last` for the last module that is on. */
    setEnabled: (id: string, enabled: boolean) => call<ModulesView>('modules_set_enabled', { id, enabled }),
    /** The answer to the first start's question: the modules to keep on. */
    choose: (enabled: string[]) => call<ModulesView>('modules_choose', { enabled }),
  },
};

/** Emitted with `ModulesView` whenever the modules that are on change. */
export const MODULES_CHANGED_EVENT = 'modules://changed';

/** A module with a switch of its own and whether it is on. */
export interface ModuleState {
  id: string;
  enabled: boolean;
}

export interface ModulesView {
  /** In the order of the product. */
  modules: ModuleState[];
  /** A new data file: the user has not said yet which modules to use. */
  first_run: boolean;
}

export interface TraySettings {
  minimize_to_tray: boolean;
  close_to_tray: boolean;
  start_hidden: boolean;
}

/**
 * Why the app started without its data. `db_foreign`: the file at `path` is not a Veydan 5
 * database. `db_dev_schema`: it is one, with the schema of another build.
 */
export interface StartError {
  code: 'db_foreign' | 'db_dev_schema';
  /** Absolute path of the file that was left untouched */
  path: string;
}

export interface SyncConfig {
  enabled: boolean;
  backend: 'folder' | 's3' | 'webdav' | string;
  folder_path: string;
  s3: {
    endpoint: string;
    region: string;
    bucket: string;
    prefix: string;
    access_key: string;
    /** Not returned by the backend. On save: null/undefined = keep stored, '' = clear. */
    secret_key?: string | null;
    has_secret_key: boolean;
    path_style: boolean;
  };
  webdav: {
    url: string;
    username: string;
    /** Same three-state rule as `s3.secret_key`. */
    password?: string | null;
    has_password: boolean;
  };
  interval_sec: number;
  /** Replicate firefox-profile directories, not only metadata. */
  profile_files: boolean;
  device_name: string;
  /** Device-local chunked transfer settings; new files only. */
  large_files: LargeFileSettings;
}

export interface LargeFileSettings {
  /** 1..64 */
  chunk_mib: number;
  /** 1..8 */
  parallelism: number;
  resume: boolean;
}

export const LARGE_FILE_LIMITS = { chunkMib: [1, 64], parallelism: [1, 8] } as const;

/** Rough peak RAM of a transfer: one buffer per in-flight chunk plus crypto copies. */
export function largeFilePeakMib(s: LargeFileSettings): number {
  return s.chunk_mib * (s.parallelism + 1) * 2;
}

export interface SyncLease {
  profile_id: string;
  device_id: string;
  device_name: string;
  own: boolean;
}

export interface SyncProgress {
  phase: string;
  percent: number;
  current: number;
  total: number;
  detail: string;
}

export interface StorageDevice {
  id: string;
  name: string;
  own: boolean;
}

export interface SyncDebugEntry {
  seq: number;
  at: string;
  cycle: number;
  source: string;
  level: string;
  step: string;
  message: string;
}

export interface SyncDebugState {
  enabled: boolean;
  entries: SyncDebugEntry[];
}

export interface SyncStatus {
  enabled: boolean;
  joined: boolean;
  running: boolean;
  vault_id: string | null;
  device_id: string;
  peers: number;
  last_run: string | null;
  last_error: string | null;
  last_warning: string | null;
  conflicts: { note_id: string; title: string }[];
  /** Profiles whose files changed on both sides; `note_id` holds the profile id. */
  profile_conflicts: { note_id: string; title: string }[];
  profile_leases: SyncLease[];
  /** Remote ops received in the last cycle. */
  last_applied: number | null;
  /** Filled by the background sync cycle. */
  storage_devices: StorageDevice[];
  gc_last: string | null;
  blobs_total: number | null;
  blobs_removed_last_gc: number | null;
  /** Chunked large-file objects (manifests + chunks) after the last cleanup. */
  lf_total: number | null;
  lf_removed_last_gc: number | null;
}

/**
 * The tray menu's words in the active locale, by key: the shell's own (`show`,
 * `hide`, `quit`, `tooltip`) and those of the modules (`ModuleDef.tray`). A
 * key nobody sends keeps its English default in the backend.
 */
export type TrayLabels = Record<string, string>;
