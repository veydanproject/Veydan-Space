// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Payloads of the Tor module's commands (crates/tor).

export interface TorStatus {
  installed: boolean;
  /** How the files got there: the app's download or the user's hands. */
  source: 'download' | 'manual' | null;
  /** Version of the Tor Expert Bundle. */
  version: string | null;
  tor_version: string | null;
  /** The bundle version pinned in this release of the app. */
  pinned_version: string;
  update_available: boolean;
  path: string | null;
  install_dir: string;
}

export type TorDownloadState =
  | { state: 'idle' }
  | { state: 'downloading'; downloaded: number; total: number; percent: number }
  | { state: 'done'; version: string }
  | { state: 'failed'; error: string };

export type TorArchiveResult =
  | { state: 'installed'; version: string | null }
  | { state: 'unknown_hash'; sha256: string };

export type BridgeMode = 'none' | 'builtin' | 'custom';

export interface TorUpstream {
  kind: 'socks5' | 'https';
  host: string;
  port: number;
  username: string;
  password: string;
}

/** The settings of the Tor daemon: local to this computer, shared by all instances. */
export interface TorSettings {
  bridges: { mode: BridgeMode; builtin: string; lines: string[] };
  upstream: TorUpstream | null;
  reachable_ports: number[];
  exclude_countries: string[];
  strict_exclude: boolean;
  start_with_app: boolean;
  idle_minutes: number;
  external_socks_port: number | null;
  extra_torrc: string[];
}

export type InstanceState = 'starting' | 'ready' | 'restarting' | 'stopping' | 'failed';

/** One running `tor` process; `any` is the one with no exit countries. */
export interface InstanceInfo {
  key: string;
  exit: string[];
  state: InstanceState;
  /** 0..100 */
  bootstrap: number;
  summary: string;
  socks_port: number | null;
  consumers: number;
  /** Started by hand: it is not stopped for being idle. */
  kept: boolean;
  /** The settings changed while it was in use; it takes them at its next start. */
  restart_needed: boolean;
  error: string | null;
}
