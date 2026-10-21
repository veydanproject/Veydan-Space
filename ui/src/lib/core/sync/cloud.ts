// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// «Veydan Cloud» in the sync settings (internal/vcloud-plan.md 4.5, 4.7):
// the backend `cloud` is offered where the module cloud is on and the
// identity has an account; without one, a way to activate it. The core
// does not import the module and does not name its commands: the module
// gives the two questions the settings ask (setCloudSource), so the build
// of a product without it carries no name of a command it has not, and
// they are asked only where the registry has the module switched on.

import { api } from '$lib/core/api';
import { registry } from '$lib/core/registry';

/** The value of `sync_backend` for Veydan Cloud. */
export const CLOUD_BACKEND = 'cloud';
/** The module that gives the backend. */
export const CLOUD_MODULE = 'cloud';

/** A quiet line in the settings from here (percent of a pool). */
export const NOTICE_PERCENT = 70;
/** A warning, and the banner of the module. */
export const WARN_PERCENT = 90;
/** The pool is full: files no longer sync. */
export const FULL_PERCENT = 100;

/** The free plan: files of profiles off, a question before they go. */
export const FREE_PLAN = 'free';

/** What the core reads of `cloud_status` (the module's answer has more). */
export interface CloudStatusView {
  configured: boolean;
  account: { plan: string } | null;
  plan: string | null;
  /** Whose servers the messenger uses; `own` and `none` have no Veydan Cloud. */
  servers?: 'none' | 'unchosen' | 'own' | 'veydan';
  /** What this device keeps of the account when the server did not answer. */
  kept?: { plan?: string } | null;
}

/** One pool of `cloud_usage`. */
export interface CloudPoolView {
  /**
   * `text`, `files`; `media` and `total` (the shared storage of the plan:
   * text, files and media together) when the plan sets them.
   */
  pool: string;
  used: number;
  limit: number;
  percent: number;
  level: number;
}

/** What the core reads of `cloud_usage`. */
export interface CloudUsageView {
  pools: CloudPoolView[];
  level: number;
  profile_files_estimate: number;
  on_cloud: boolean;
}

/** What the module cloud answers the sync settings with: its own commands, named by the module. */
export interface CloudSource {
  status: () => Promise<CloudStatusView>;
  usage: () => Promise<CloudUsageView>;
  /**
   * Makes sure the identity has an account: starts the activation of the
   * free plan where there is none and resolves when it ended, refusing with
   * the words of the module when it failed.
   */
  ensure?: () => Promise<void>;
}

let source: CloudSource | null = null;

/** The module cloud gives its answers when it is loaded; null takes them back. */
export function setCloudSource(next: CloudSource | null): void {
  source = next;
}

/** `option`: offer the backend; `activate`: offer the activation; `none`: say nothing. */
export type CloudChoice = 'option' | 'activate' | 'none';

/** What the sync settings know of Veydan Cloud. */
export interface CloudView {
  choice: CloudChoice;
  /** The plan of the account (`free`, `plus`, …), when there is one. */
  plan: string | null;
  usage: CloudUsageView | null;
}

/** The plan of the account, from the server or from what the device kept. */
export function planOf(status: CloudStatusView | null): string | null {
  return status?.plan ?? status?.account?.plan ?? status?.kept?.plan ?? null;
}

/**
 * Whether the form offers Veydan Cloud: never without the module or on the
 * user's own servers (plan 4.5); the backend once there is an account; the
 * activation before.
 */
export function cloudChoice(moduleOn: boolean, status: CloudStatusView | null): CloudChoice {
  if (!moduleOn || !status) return 'none';
  if (status.servers === 'own' || status.servers === 'none') return 'none';
  return planOf(status) !== null ? 'option' : 'activate';
}

/**
 * The fullest pool, the one the line and the banner speak of. The shared
 * storage wins a tie: the files of a plan with one have the same limit
 * (the server fills it so), and a full total stops more than the files.
 */
export function fullestPool(usage: CloudUsageView | null): CloudPoolView | null {
  if (!usage || usage.pools.length === 0) return null;
  return usage.pools.reduce((a, b) => (b.percent > a.percent || (b.percent === a.percent && b.pool === 'total') ? b : a));
}

/** How the settings show the room: nothing under 70 %, quietly to 90 %, a warning to 100 %, full. */
export function usageTone(level: number): 'none' | 'quiet' | 'warn' | 'full' {
  if (level >= FULL_PERCENT) return 'full';
  if (level >= WARN_PERCENT) return 'warn';
  if (level >= NOTICE_PERCENT) return 'quiet';
  return 'none';
}

/** The files of profiles on the free plan wait for a yes (plan 4.7). */
export function asksForProfileFiles(backend: string, plan: string | null): boolean {
  return backend === CLOUD_BACKEND && plan === FREE_PLAN;
}

/** The pool `files` and its limit, for the estimate of the question. */
export function filesLimit(usage: CloudUsageView | null): number | null {
  const files = usage?.pools.find((p) => p.pool === 'files');
  return files && files.limit > 0 ? files.limit : null;
}

/**
 * A host of the storage refused the identity as one without an account
 * (`403 no_account`, spec/vstore.md §2). The hosts learn of an account from
 * the signed table the cloud sends them (§3): one issue in half a minute,
 * then the hosts read it — so for about a minute after its activation a new
 * account is refused there, though it exists.
 */
export function cloudNotReady(e: unknown): boolean {
  const text = typeof e === 'string' ? e : e != null && typeof e === 'object' && 'message' in e ? String((e as { message: unknown }).message) : '';
  return /\bno_account\b/.test(text);
}

/** Where `connectCloudVault` is: the account, its storage, the vault of this device. */
export type VaultStage = 'account' | 'storage' | 'device';

/** How `connectCloudVault` waits for the storage of a new account, and what it tells of its way. */
export interface VaultWait {
  /** Called as each stage begins, for the screen that shows the way. */
  onStage?: (stage: VaultStage) => void;
  /** Called once, when the first host said it does not know the account yet. */
  onWait?: () => void;
  /** For how long it asks again before the refusal is the answer. */
  limitMs?: number;
  /** The pause between two tries. */
  retryMs?: number;
}

/**
 * Turns on sync through Veydan Cloud: the account first (the free plan is
 * activated where the identity has none), then the backend and the vault of
 * the account — joined when another device made it already, created
 * otherwise. No passphrase: it comes from the identity. On the free plan
 * the files of profiles stay off until the person says yes (plan 4.7).
 * An account made a moment ago is not known to the storage yet
 * (`cloudNotReady`): the vault is asked again until it is, for two minutes
 * at most. The refusal comes before anything is written, so asking again
 * repeats nothing.
 */
export async function connectCloudVault(plan: string | null, wait: VaultWait = {}): Promise<void> {
  wait.onStage?.('account');
  await ensureCloudAccount();
  wait.onStage?.('storage');
  const cfg = await api.sync.getConfig();
  const profile_files = asksForProfileFiles(CLOUD_BACKEND, plan) ? false : cfg.profile_files;
  await api.sync.setConfig({ ...cfg, backend: CLOUD_BACKEND, profile_files });
  const until = Date.now() + (wait.limitMs ?? 120_000);
  let told = false;
  for (;;) {
    try {
      const found = await api.sync.probe();
      wait.onStage?.('device');
      if (found === 'vault') await api.sync.joinVault('');
      else await api.sync.createVault('');
      return;
    } catch (e) {
      if (!cloudNotReady(e) || Date.now() >= until) throw e;
      // Back to the storage: the host that refused does not know the account yet.
      wait.onStage?.('storage');
      if (!told) wait.onWait?.();
      told = true;
      await new Promise((resolve) => setTimeout(resolve, wait.retryMs ?? 4_000));
    }
  }
}

/**
 * Whether the account has a vault already (another device made it): the
 * first start then offers to connect this device instead of turning sync
 * on. The backend of a device that syncs nowhere becomes `cloud` for the
 * question; a device that is joined, or has a storage of its own filled in,
 * is not asked. A host that does not answer counts as no vault.
 */
export async function cloudVaultExists(): Promise<boolean> {
  try {
    const [status, cfg] = await Promise.all([api.sync.status(), api.sync.getConfig()]);
    if (status.joined) return false;
    if (cfg.backend !== CLOUD_BACKEND) {
      if (cfg.folder_path.trim() || cfg.s3.endpoint.trim() || cfg.s3.bucket.trim() || cfg.webdav.url.trim()) return false;
      await api.sync.setConfig({ ...cfg, backend: CLOUD_BACKEND });
    }
    return (await api.sync.probe()) === 'vault';
  } catch {
    return false;
  }
}

/** The account of Veydan Cloud is there by the end; nothing where the module gives no way to make one. */
export async function ensureCloudAccount(): Promise<void> {
  await source?.ensure?.();
}

/** The status and, with an account, the usage; nothing where the module is off. */
export async function loadCloud(): Promise<CloudView> {
  const none: CloudView = { choice: 'none', plan: null, usage: null };
  if (!source || !registry.enabled(CLOUD_MODULE)) return none;
  let status: CloudStatusView | null = null;
  try {
    status = await source.status();
  } catch {
    return none;
  }
  const choice = cloudChoice(true, status);
  let usage: CloudUsageView | null = null;
  if (choice === 'option') {
    try {
      usage = await source.usage();
    } catch {
      usage = null;
    }
  }
  return { choice, plan: planOf(status), usage };
}
