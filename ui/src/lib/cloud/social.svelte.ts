// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Social Recovery on the screens (internal/cloud-recovery.md §8, §11): the
// trusted people of the owner and their health, «Помочь другу» of a
// trusted person, «Через доверенных людей» of a clean install, and the
// alerts from the cards of the bot. The shares, the keys and the relays
// stay in Rust (`cloud_trustees_*`, `cloud_help_*`, `cloud_recover_*`,
// `cloud_recovery_*`).

import {
  asCloudError,
  ALERTS_EVENT,
  IDLE_REQUEST,
  NO_TRUSTEES,
  socialApi,
  TRUSTEES_EVENT,
  type CloudAlert,
  type CloudCancelled,
  type CloudContact,
  type CloudError,
  type CloudHelpView,
  type CloudRecovered,
  type CloudRequestView,
  type CloudRestoreSecret,
  type CloudTrustee,
  type CloudTrusteeHealth,
  type CloudTrustees,
} from '$lib/cloud/api';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** The bounds of the picker (cloud-recovery.md §8.1). */
export const MIN_PEOPLE = 3;
export const MAX_PEOPLE = 7;
export const DEFAULT_K = 3;

/** The key of the text of a health (§8.3). */
export function healthKey(health: CloudTrusteeHealth): `cloud_trustee_${CloudTrusteeHealth}` {
  return `cloud_trustee_${health}`;
}

/** Whole days between `then` and `now`, never negative. */
export function daysBetween(then: number, now: number): number {
  return Math.max(0, Math.floor((now - then) / 86_400));
}

/** The text for an error of this module, by its code; null: its own message. */
export type SocialErrorKey =
  | 'cloud_social_err_trustees_count'
  | 'cloud_social_err_threshold'
  | 'cloud_social_err_not_approved'
  | 'cloud_social_err_rk_not_issued'
  | 'cloud_social_err_not_set_up'
  | 'cloud_social_err_shares_not_acked'
  | 'cloud_help_err_bad_npub'
  | 'cloud_help_err_no_share'
  | 'cloud_help_err_no_request'
  | 'cloud_help_err_code_mismatch'
  | 'cloud_help_err_confirm_required'
  | 'cloud_help_err_request_closed'
  | 'cloud_recover_err_not_configured'
  | 'cloud_recover_err_request_open'
  | 'cloud_recover_err_frozen'
  | 'cloud_recover_err_not_collected'
  | 'cloud_recover_err_expired'
  | 'cloud_err_rate_limited'
  | 'cloud_err_no_host'
  | 'cloud_err_locked'
  | 'cloud_err_messenger_stopped'
  | 'cloud_link_err_no_relay';

const ERROR_KEYS: Record<string, SocialErrorKey> = {
  trustees_count: 'cloud_social_err_trustees_count',
  threshold: 'cloud_social_err_threshold',
  not_approved: 'cloud_social_err_not_approved',
  rk_not_issued: 'cloud_social_err_rk_not_issued',
  not_set_up: 'cloud_social_err_not_set_up',
  shares_not_acked: 'cloud_social_err_shares_not_acked',
  bad_npub: 'cloud_help_err_bad_npub',
  no_share: 'cloud_help_err_no_share',
  no_request: 'cloud_help_err_no_request',
  code_mismatch: 'cloud_help_err_code_mismatch',
  confirm_required: 'cloud_help_err_confirm_required',
  request_closed: 'cloud_help_err_request_closed',
  not_configured: 'cloud_recover_err_not_configured',
  request_open: 'cloud_recover_err_request_open',
  frozen: 'cloud_recover_err_frozen',
  not_collected: 'cloud_recover_err_not_collected',
  request_expired: 'cloud_recover_err_expired',
  rate_limited: 'cloud_err_rate_limited',
  no_host: 'cloud_err_no_host',
  locked: 'cloud_err_locked',
  messenger_stopped: 'cloud_err_messenger_stopped',
  no_relay: 'cloud_link_err_no_relay',
};

export function socialErrorKey(error: CloudError | null): SocialErrorKey | null {
  return error ? (ERROR_KEYS[error.code] ?? null) : null;
}

/** `npub` and `code` of `/cloud/help?npub=…&code=…`, as the card of a link opens it. */
export function helpInputOf(search: string): string {
  const q = new URLSearchParams(search);
  const npub = q.get('npub')?.trim() ?? '';
  const code = q.get('code')?.trim() ?? '';
  if (!npub) return '';
  return code ? `veydan://recover/${npub}?c=${code}` : npub;
}

/** `npub1…` shortened for a line: the first 12 and the last 4 characters. */
export function shortKey(key: string): string {
  return key.length > 20 ? `${key.slice(0, 12)}…${key.slice(-4)}` : key;
}

// ── «Доверенные люди» ────────────────────────────────────────────────────

export class TrusteesStore {
  status = $state<CloudTrustees>(NO_TRUSTEES);
  approved = $state<CloudContact[]>([]);
  busy = $state(false);
  error = $state<CloudError | null>(null);
  /** The chosen keys of the picker. */
  chosen = $state<string[]>([]);
  k = $state(DEFAULT_K);
  private unlisten: (() => void)[] = [];

  async load(): Promise<void> {
    try {
      [this.status, this.approved] = await Promise.all([socialApi.trustees(), socialApi.approved()]);
      this.error = null;
    } catch (e) {
      this.error = asCloudError(e);
    }
  }

  /** Whether the picker may send: 3 to 7 people, 2 ≤ k ≤ n. */
  get ready(): boolean {
    const n = this.chosen.length;
    return n >= MIN_PEOPLE && n <= MAX_PEOPLE && this.k >= 2 && this.k <= n;
  }

  toggle(pubkey: string): void {
    if (this.chosen.includes(pubkey)) this.chosen = this.chosen.filter((p) => p !== pubkey);
    else if (this.chosen.length < MAX_PEOPLE) this.chosen = [...this.chosen, pubkey];
    if (this.k > this.chosen.length) this.k = Math.max(2, this.chosen.length);
  }

  /** The picker starts from the set in place. */
  startFromStatus(): void {
    this.chosen = this.status.people.map((p) => p.pubkey);
    this.k = this.status.configured ? this.status.k : DEFAULT_K;
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

  /** The chosen people with the threshold: the shares go. */
  async save(): Promise<boolean> {
    const people = this.chosen
      .map((pubkey) => this.approved.find((c) => c.pubkey === pubkey))
      .filter((c): c is CloudContact => c !== undefined);
    const status = await this.run(() => socialApi.setup(this.k, people));
    if (status) this.status = status;
    return status !== null;
  }

  /** «Заменить» one person: the picker without them. */
  replace(pubkey: string): void {
    this.startFromStatus();
    this.chosen = this.chosen.filter((p) => p !== pubkey);
  }

  async disable(): Promise<boolean> {
    const revoked = await this.run(() => socialApi.disable());
    if (revoked !== null) await this.load();
    return revoked !== null;
  }

  async check(): Promise<void> {
    const status = await this.run(() => socialApi.check());
    if (status) this.status = status;
  }

  /** Follows the state while the page is open; returns the way to stop. */
  async listen(): Promise<() => void> {
    if (!isTauri) return () => {};
    const { listen } = await import('@tauri-apps/api/event');
    this.unlisten.push(await listen<CloudTrustees>(TRUSTEES_EVENT, (e) => (this.status = e.payload)));
    return () => {
      for (const stop of this.unlisten.splice(0)) stop();
    };
  }
}

/** The line of «Восстановление»: «Доверенные: k из n, доли живы» or none. */
export function trusteesLine(status: CloudTrustees | null, words: { of: (k: number, n: number) => string; alive: (alive: number, n: number) => string }): string {
  if (!status?.configured) return '';
  const line = words.of(status.k, status.n);
  return status.alive > 0 ? `${line} · ${words.alive(status.alive, status.n)}` : line;
}

export function trusteeOf(status: CloudTrustees, pubkey: string): CloudTrustee | undefined {
  return status.people.find((p) => p.pubkey === pubkey);
}

// ── «Помочь другу восстановить доступ» ───────────────────────────────────

export type HelpPhase = 'input' | 'looking' | 'shown' | 'sending' | 'sent' | 'closed';

export class HelpStore {
  phase = $state<HelpPhase>('input');
  view = $state<CloudHelpView | null>(null);
  error = $state<CloudError | null>(null);
  /** The mandatory checkbox: «говорил(а) голосом или лично». */
  spoke = $state(false);

  async lookup(input: string): Promise<void> {
    this.error = null;
    if (!input.trim()) {
      this.error = { code: 'bad_npub', message: 'empty', request_id: null };
      return;
    }
    this.phase = 'looking';
    this.spoke = false;
    try {
      this.view = await socialApi.helpLookup(input);
      this.phase = 'shown';
    } catch (e) {
      this.error = asCloudError(e);
      this.view = null;
      this.phase = 'input';
    }
  }

  /** «Подтвердить»: only with the checkbox, only for an open request whose code is not contradicted. */
  async confirmRequest(): Promise<void> {
    if (!this.view?.can_confirm || !this.spoke) {
      this.error = { code: 'confirm_required', message: 'the checkbox', request_id: null };
      return;
    }
    this.error = null;
    this.phase = 'sending';
    try {
      await socialApi.helpConfirm(true);
      this.phase = 'sent';
    } catch (e) {
      this.error = asCloudError(e);
      this.phase = 'shown';
    }
  }

  /** «Это не он — закрыть». */
  async close(): Promise<void> {
    try {
      await socialApi.helpClose();
    } catch {
      // Nothing was kept.
    }
    this.view = null;
    this.spoke = false;
    this.error = null;
    this.phase = 'closed';
  }

  reset(): void {
    this.view = null;
    this.spoke = false;
    this.error = null;
    this.phase = 'input';
  }
}

// ── «Через доверенных людей» ─────────────────────────────────────────────

export class RecoverStore {
  view = $state<CloudRequestView>(IDLE_REQUEST);
  error = $state<CloudError | null>(null);
  busy = $state(false);
  recovered = $state<CloudRecovered | null>(null);

  /** The key of the account: the proof, the request, the code. */
  async start(npub: string): Promise<void> {
    this.error = null;
    this.recovered = null;
    if (!npub.trim()) {
      this.error = { code: 'bad_npub', message: 'empty', request_id: null };
      return;
    }
    this.busy = true;
    this.view = { ...IDLE_REQUEST, phase: 'preparing', npub };
    try {
      this.view = await socialApi.recoverStart(npub);
    } catch (e) {
      this.error = asCloudError(e);
      this.view = { ...IDLE_REQUEST, phase: 'failed', npub, error: this.error };
    } finally {
      this.busy = false;
    }
  }

  /** One look at the request while it waits or prepares. */
  async poll(): Promise<void> {
    if (!['waiting', 'preparing'].includes(this.view.phase)) return;
    try {
      this.view = await socialApi.recoverPoll();
    } catch (e) {
      this.error = asCloudError(e);
    }
  }

  /**
   * The key is together: the restore with the new secret of the lock;
   * `withoutVault` after `vault_unreachable`, the identity and the lock alone.
   */
  async finish(secret: CloudRestoreSecret, withoutVault = false): Promise<boolean> {
    this.error = null;
    this.busy = true;
    try {
      this.recovered = await socialApi.recoverFinish(secret, withoutVault);
      this.view = { ...this.view, phase: 'restored' };
      return true;
    } catch (e) {
      this.error = asCloudError(e);
      return false;
    } finally {
      this.busy = false;
    }
  }

  async cancel(): Promise<void> {
    try {
      await socialApi.recoverCancel();
    } catch {
      // The request expires on the server by itself.
    }
    this.view = IDLE_REQUEST;
    this.error = null;
  }
}

// ── The alerts of the owner's live device ────────────────────────────────

/** The refusals of a cancel that say the request is over anyway (cloud-api.md 5.4). */
const OVER_CODES = ['request_closed', 'not_found'];

export class AlertsStore {
  alerts = $state<CloudAlert[]>([]);
  busy = $state(false);
  error = $state<CloudError | null>(null);
  /** The answer of the last cancel, with the freeze. */
  cancelled = $state<CloudCancelled | null>(null);
  /**
   * The requests this device knows to be over, by `request_id`: cancelled
   * here (the answer, with the freeze), cancelled on another device (its
   * alert was taken down, `cancelled` without a date), or refused by the
   * server as over (`closed`). The card of such a request in the chat with
   * the bot offers no «Это не я — отменить». Kept for the run of the app.
   */
  closed = $state<Record<string, CloudCancelled>>({});
  /** Alerts closed by hand here: their going says nothing of the request. */
  private dismissed = new Set<string>();
  private unlisten: (() => void)[] = [];

  /** The alerts as the module has them now; one of a request that went without being closed here was cancelled elsewhere. */
  take(next: CloudAlert[]): void {
    for (const a of this.alerts) {
      const id = a.request_id;
      if (!id || this.dismissed.has(a.id) || id in this.closed || next.some((n) => n.request_id === id)) continue;
      this.closed = { ...this.closed, [id]: { state: 'cancelled', frozen_until: null } };
    }
    this.alerts = next;
  }

  async load(): Promise<void> {
    try {
      this.alerts = await socialApi.alerts();
    } catch {
      this.alerts = [];
    }
  }

  /** «Это не я — отменить». */
  async cancel(requestId: string): Promise<boolean> {
    this.busy = true;
    this.error = null;
    try {
      this.cancelled = await socialApi.cancelRequest(requestId);
      this.closed = { ...this.closed, [requestId]: this.cancelled };
      this.alerts = this.alerts.filter((a) => a.request_id !== requestId);
      return true;
    } catch (e) {
      this.error = asCloudError(e);
      // Over on the server: nothing is left to cancel, here or from the card in the chat.
      if (OVER_CODES.includes(this.error.code)) this.closed = { ...this.closed, [requestId]: { state: 'closed', frozen_until: null } };
      return false;
    } finally {
      this.busy = false;
    }
  }

  async dismiss(id: string): Promise<void> {
    this.dismissed.add(id);
    this.alerts = this.alerts.filter((a) => a.id !== id);
    try {
      await socialApi.dismissAlert(id);
    } catch {
      // Gone from the screen anyway.
    }
  }

  async listen(): Promise<() => void> {
    if (!isTauri) return () => {};
    const { listen } = await import('@tauri-apps/api/event');
    this.unlisten.push(await listen<CloudAlert[]>(ALERTS_EVENT, (e) => this.take(e.payload)));
    return () => {
      for (const stop of this.unlisten.splice(0)) stop();
    };
  }
}

export const alertsStore = new AlertsStore();
