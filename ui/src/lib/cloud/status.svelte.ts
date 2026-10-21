// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the page shows of Veydan Cloud on this device: read when the page
// opens (cloud-api.md 4.3: by opening, not by a timer), kept while it is
// open, and moved by the events of an activation and of a deletion.

import { api, call } from '$lib/core/api';
import {
  ACTIVATION_EVENT,
  asCloudError,
  cloudApi,
  DELETED_EVENT,
  IDLE_ACTIVATION,
  type CloudActivation,
  type CloudDeleteReceipt,
  type CloudError,
  type CloudPlans,
  type CloudStatus,
} from '$lib/cloud/api';
import type { DeleteForm } from '$lib/cloud/account';
import { usageApi, type CloudCategory } from '$lib/cloud/usage';
import { ERR_SYNC_RUNNING } from '$lib/core/join-error';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** The line under the page's sentence, from `cloud_status`. */
export function statusLine(status: CloudStatus | null, words: { notConnected: string; name: string }): string {
  if (!status) return '';
  if (!status.configured) return words.notConnected;
  return status.plan ? `${words.name} · ${status.plan}` : words.name;
}

/** The line of a failure: its code first, the words to branch on, then what the server said. */
export function errorLine(error: CloudError | null): string {
  if (!error) return '';
  return error.message ? `${error.code}: ${error.message}` : error.code;
}

/** The status of a `cloud_status` that failed: no answer, the refusal, what was kept before. */
export function failedStatus(error: CloudError, before: CloudStatus | null): CloudStatus {
  return {
    configured: false,
    account: null,
    plan: null,
    servers: 'veydan',
    kept: before?.kept ?? null,
    error,
    activation: before?.activation,
  };
}

/** Whether an activation is under way. */
export function activating(a: CloudActivation): boolean {
  return a.phase === 'asking' || a.phase === 'solving' || a.phase === 'activating';
}

class CloudPageState {
  status = $state<CloudStatus | null>(null);
  /** The last failure of a command of the page, structured: the screens branch on `code`. */
  error = $state<CloudError | null>(null);
  activation = $state<CloudActivation>(IDLE_ACTIVATION);
  /** A command of the page is under way. */
  busy = $state(false);
  /** The receipt of an account deleted, here or on another device. */
  deleted = $state<CloudDeleteReceipt | null>(null);
  /** The plans on offer; null while unknown, and where the host has no list to give. */
  plans = $state<CloudPlans | null>(null);
  /** What takes the room of the vault, as its host counts it now. */
  categories = $state<CloudCategory[]>([]);
  private unlisten: (() => void)[] = [];
  /** A host answered that the identity has no account (after a deletion, before an activation). */
  private sawNoAccount = false;

  /**
   * Reads `cloud_status`. When the command itself fails (the messenger
   * off, no identity) the page leaves «loading» for the screen of a host
   * that did not answer, with the words of the refusal and a retry.
   */
  async load(): Promise<void> {
    try {
      this.status = await cloudApi.status();
      if (this.status.activation) this.activation = this.status.activation;
      this.error = null;
      // A host that says the identity has no account is remembered: an account after that is a new one,
      // and takes the place of the receipt of the one deleted before it.
      if (this.status.configured && !this.status.account) this.sawNoAccount = true;
      else if (this.status.account && this.sawNoAccount) {
        this.sawNoAccount = false;
        this.deleted = null;
      }
      await Promise.all([this.overlayLiveUsage(), this.loadPlans()]);
    } catch (e) {
      this.status = failedStatus(asCloudError(e), this.status);
      this.error = null;
    }
  }

  /**
   * The pools of the vault as its host counts them now (`cloud_usage`), laid
   * over the numbers the account carries: those come from the hourly report
   * of the storage hosts to vcloud and lag by up to an hour, so a vault
   * just created showed «0 B» (E1). The media stay the account's unless the
   * host counts them from its index (`media.known`, spec/vstore.md §6). The
   * shared storage is not laid over: `poolsOf` sums it again from these. A
   * host that does not answer leaves the account as it is.
   */
  private async overlayLiveUsage(): Promise<void> {
    const account = this.status?.account;
    if (!account) {
      this.categories = [];
      return;
    }
    try {
      const live = await usageApi.usage();
      // A later read of the page (a key, a refresh, a deletion) may have come first: its account stands.
      if (this.status?.account !== account) return;
      this.categories = live.categories ?? [];
      const files = live.pools.find((p) => p.pool === 'files');
      const text = live.pools.find((p) => p.pool === 'text');
      const liveMedia = live.usage?.media?.known ? live.usage.media : null;
      if (!files && !text && !liveMedia) return;
      const vault = account.usage?.vault ?? { files_bytes: 0, text_bytes: 0 };
      const media = liveMedia
        ? { upload_today_bytes: account.usage?.media?.upload_today_bytes ?? 0, storage_bytes: liveMedia.used }
        : (account.usage?.media ?? null);
      const usage = {
        as_of: account.usage?.as_of ?? null,
        media,
        vault: { files_bytes: files?.used ?? vault.files_bytes, text_bytes: text?.used ?? vault.text_bytes },
      };
      this.status = { ...this.status!, account: { ...account, usage } };
    } catch {
      // The numbers of the account stand.
    }
  }

  /**
   * The plans on offer, asked only of the project's hosts (a product on its
   * own servers sends them nothing) and once per run: the list changes when
   * the owner changes it, not while a page is open. A host that fails
   * leaves the page without the choice, never without the account.
   */
  private async loadPlans(): Promise<void> {
    if (this.plans || (this.status?.servers ?? 'veydan') !== 'veydan' || !this.status?.configured) return;
    try {
      const plans = await cloudApi.plans();
      if (Array.isArray(plans?.plans) && plans.plans.length > 0) this.plans = plans;
    } catch {
      // No list: the page keeps the key of activation alone.
    }
  }

  private async run<T>(work: () => Promise<T>): Promise<T | null> {
    this.busy = true;
    this.error = null;
    try {
      return await work();
    } catch (e) {
      this.error = asCloudError(e);
      return null;
    } finally {
      this.busy = false;
    }
  }

  /** Asks the server for the account now. */
  async refresh(): Promise<void> {
    await this.run(() => cloudApi.refresh());
    if (!this.error) await this.load();
  }

  /** Starts the activation of the free plan; the events carry it to its end. */
  async activate(): Promise<void> {
    const started = await this.run(() => cloudApi.activateFree());
    if (started) this.activation = started;
  }

  /** Redeems a key of activation; true when it was taken. */
  async redeem(key: string): Promise<boolean> {
    const account = await this.run(() => cloudApi.redeem(key));
    if (!account) return false;
    await this.load();
    return true;
  }

  /**
   * Deletes the account (vcloud-plan.md 4.6). After the receipt: the sync
   * of this device leaves the backend `cloud`, and, when asked, the
   * identity of the messenger and its data on this device go too.
   */
  async deleteAccount(form: DeleteForm): Promise<CloudDeleteReceipt | null> {
    const receipt = await this.run(() =>
      cloudApi.deleteAccount({
        confirm: form.word,
        lockSecret: form.lockOn ? form.secret : null,
        revokeShares: form.revokeShares,
      }),
    );
    if (!receipt) return null;
    this.deleted = receipt;
    let leaveFailed: CloudError | null = null;
    try {
      await leaveCloudSync();
    } catch (e) {
      leaveFailed = asCloudError(e);
    }
    let wipeFailed: CloudError | null = null;
    if (form.wipeDevice) {
      try {
        await call<void>('messenger_identity_delete');
      } catch (e) {
        wipeFailed = asCloudError(e);
      }
    }
    await this.load();
    // The account is gone but the identity stayed on this device: said after the page was read again, which clears errors.
    if (wipeFailed ?? leaveFailed) this.error = wipeFailed ?? leaveFailed;
    return receipt;
  }

  /** Follows the events of the module while a page is open; returns the way to stop. */
  async listen(): Promise<() => void> {
    if (!isTauri) return () => {};
    const { listen } = await import('@tauri-apps/api/event');
    this.unlisten.push(
      await listen<CloudActivation>(ACTIVATION_EVENT, (e) => {
        this.activation = e.payload;
        if (e.payload.phase === 'done' || e.payload.phase === 'failed') void this.load();
      }),
      await listen<CloudDeleteReceipt>(DELETED_EVENT, (e) => {
        this.deleted = e.payload;
        void (async () => {
          let failed: CloudError | null = null;
          try {
            await leaveCloudSync();
          } catch (err) {
            failed = asCloudError(err);
          }
          await this.load();
          // Said after the page was read again, which clears errors.
          if (failed) this.error = failed;
        })();
      }),
    );
    return () => {
      for (const stop of this.unlisten.splice(0)) stop();
    };
  }
}

/** The backend of sync that Veydan Cloud gives (`sync_backend`). */
const CLOUD_BACKEND = 'cloud';

/** How often the leave is asked again while a cycle holds the binding; the command itself waits for it. */
const LEAVE_TRIES = 3;

/**
 * The sync of this device lets go of the backend `cloud` (plan 4.6:
 * `sync_leave`). A leave refused while a cycle runs is asked again; one that
 * still fails is thrown, so the device is never left bound in silence.
 */
export async function leaveCloudSync(): Promise<void> {
  let joined = false;
  try {
    const config = await api.sync.getConfig();
    if (config?.backend !== CLOUD_BACKEND) return;
    joined = (await api.sync.status())?.joined ?? false;
  } catch {
    // A product without sync, or sync not set up: nothing to leave.
    return;
  }
  for (let attempt = 1; joined; attempt++) {
    try {
      await api.sync.leave();
      return;
    } catch (e) {
      if (attempt >= LEAVE_TRIES || (e as { message?: string })?.message !== ERR_SYNC_RUNNING) throw e;
    }
  }
}

export const cloudPage = new CloudPageState();
