// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Linking a device over the relays (internal/cloud-recovery.md §6): the new
// device shows a QR code and 8 words and waits; the old one takes them,
// both compare 4 words, and the account comes over after the lock's
// password. The commands are the module's (`cloud_link_*`); the relays,
// the keys and both wraps stay in Rust.

import { call, mockCommand } from '$lib/core/api';
import { asCloudError, VAULT_UNREACHABLE, type CloudError } from '$lib/cloud/api';

/** What the new device shows (`cloud_link_start`). */
export interface LinkStart {
  t_pub: string;
  words: string[];
  /** `veydan://link/<T>/<words>`: the content of the QR code. */
  uri: string;
  qr_svg: string;
  fingerprint: string[];
  expires_at: number;
}

/** Where the link of the new device is (`cloud_link_poll`). */
export interface LinkPoll {
  state: 'idle' | 'waiting' | 'received' | 'expired';
  expires_at: number | null;
  npub: string | null;
}

/** What the old device found behind the QR code or the words (`cloud_link_resolve`). */
export interface LinkTarget {
  t_pub: string;
  fingerprint: string[];
  expires_at: number;
}

/** The account went to the new device (`cloud_link_approve`). */
export interface LinkSent {
  t_pub: string;
  expires_at: number;
}

/** The new device took the account over (`cloud_link_receive`). */
export interface LinkDone {
  npub: string;
  done_sent: boolean;
  vault_joined: boolean;
}

export const linkApi = {
  start: () => call<LinkStart>('cloud_link_start'),
  poll: () => call<LinkPoll>('cloud_link_poll'),
  receive: (password: string | null, withoutVault = false) =>
    call<LinkDone>('cloud_link_receive', { password, withoutVault }),
  resolve: (input: string) => call<LinkTarget>('cloud_link_resolve', { input }),
  approve: (password: string | null) => call<LinkSent>('cloud_link_approve', { password }),
  cancel: () => call<void>('cloud_link_cancel'),
};

// In a plain browser there are no relays: nothing is linked.
mockCommand('cloud_link_poll', { state: 'idle', expires_at: null, npub: null } satisfies LinkPoll);

/** The words of the errors a link shows, by the code of the error. */
export type LinkErrorKey =
  | 'cloud_link_err_no_relay'
  | 'cloud_link_err_expired'
  | 'cloud_link_err_not_found'
  | 'cloud_link_err_code'
  | 'cloud_link_err_wrong_password'
  | 'cloud_link_err_locked'
  | 'cloud_link_err_identity_exists'
  | 'cloud_link_err_own_servers'
  | 'cloud_link_err_stopped'
  | 'cloud_vault_unreachable'
  | 'cloud_restore_not_clean';

const ERROR_KEYS: Record<string, LinkErrorKey> = {
  no_relay: 'cloud_link_err_no_relay',
  link_expired: 'cloud_link_err_expired',
  link_not_found: 'cloud_link_err_not_found',
  link_code: 'cloud_link_err_code',
  wrong_password: 'cloud_link_err_wrong_password',
  locked: 'cloud_link_err_locked',
  identity_exists: 'cloud_link_err_identity_exists',
  no_host: 'cloud_link_err_own_servers',
  messenger_stopped: 'cloud_link_err_stopped',
  [VAULT_UNREACHABLE]: 'cloud_vault_unreachable',
  not_clean: 'cloud_restore_not_clean',
};

/** The refusals of the vault a link brings: the account can still come without it. */
const VAULT_REFUSALS = [VAULT_UNREACHABLE, 'not_clean'];

/** The key of the text for an error, or null: then its own message is shown. */
export function linkErrorKey(error: CloudError | null): LinkErrorKey | null {
  return error ? (ERROR_KEYS[error.code] ?? null) : null;
}

/** Minutes and seconds left until `expiresAt`, as `m:ss`. */
export function timeLeft(expiresAt: number, nowSecs: number): string {
  const left = Math.max(0, Math.floor(expiresAt - nowSecs));
  return `${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}`;
}

/** The SVG of a QR code as the source of an image: nothing inside it can run. */
export function qrDataUrl(svg: string): string {
  return `data:image/svg+xml;base64,${btoa(unescape(encodeURIComponent(svg)))}`;
}

/** The phases of the new device. */
export type NewPhase = 'idle' | 'starting' | 'waiting' | 'received' | 'expired' | 'signing_in' | 'done' | 'failed';

/** The new device: «Войти с другого устройства». */
export class NewDeviceLink {
  phase = $state<NewPhase>('idle');
  shown = $state<LinkStart | null>(null);
  done = $state<LinkDone | null>(null);
  error = $state<CloudError | null>(null);
  /** The account came and the lock of this device wants its password. */
  needsPassword = $state(false);
  /** The storage the other device uses is out of reach, or this device has a lock or a storage of its own: going on without it is offered. */
  vaultAway = $state(false);

  /** A new code: the earlier one of this device ends. */
  async begin(): Promise<void> {
    this.phase = 'starting';
    this.error = null;
    this.done = null;
    this.needsPassword = false;
    this.vaultAway = false;
    try {
      this.shown = await linkApi.start();
      this.phase = 'waiting';
    } catch (e) {
      this.shown = null;
      this.error = asCloudError(e);
      this.phase = 'failed';
    }
  }

  /** One look at the link; once the account came, it is taken over. */
  async poll(): Promise<void> {
    if (this.phase !== 'waiting') return;
    try {
      const poll = await linkApi.poll();
      if (poll.state === 'expired') this.phase = 'expired';
      else if (poll.state === 'idle') this.phase = 'idle';
      else if (poll.state === 'received') {
        this.phase = 'received';
        await this.receive(null);
      }
    } catch (e) {
      this.error = asCloudError(e);
    }
  }

  /**
   * The account taken over, with the storage of the other device set up
   * here; a closed lock asks for its password and a storage out of reach
   * offers to go on without it (`withoutVault`): both keep the link.
   */
  async receive(password: string | null, withoutVault = false): Promise<void> {
    this.phase = 'signing_in';
    this.error = null;
    this.vaultAway = false;
    try {
      this.done = await linkApi.receive(password, withoutVault);
      this.shown = null;
      this.needsPassword = false;
      this.phase = 'done';
    } catch (e) {
      const error = asCloudError(e);
      if (error.code === 'locked' || error.code === 'wrong_password') {
        this.needsPassword = true;
        // No password asked yet: the field is the news, not an error.
        this.error = password === null ? null : error;
      } else {
        this.vaultAway = VAULT_REFUSALS.includes(error.code);
        this.error = error;
      }
      this.phase = 'received';
    }
  }

  async cancel(): Promise<void> {
    try {
      await linkApi.cancel();
    } catch {
      // The code expires on the relays by itself.
    }
    this.shown = null;
    this.needsPassword = false;
    this.vaultAway = false;
    this.error = null;
    this.phase = 'idle';
  }
}

/** The phases of the old device. */
export type OldPhase = 'input' | 'resolving' | 'compare' | 'sending' | 'sent' | 'mismatch';

/** The old device: «Добавить устройство». */
export class OldDeviceLink {
  phase = $state<OldPhase>('input');
  target = $state<LinkTarget | null>(null);
  error = $state<CloudError | null>(null);

  /** The QR code or the 8 words lead to the new device. */
  async resolve(input: string): Promise<void> {
    this.error = null;
    if (!input.trim()) {
      this.error = { code: 'link_code', message: 'empty', request_id: null };
      return;
    }
    this.phase = 'resolving';
    try {
      this.target = await linkApi.resolve(input);
      this.phase = 'compare';
    } catch (e) {
      this.error = asCloudError(e);
      this.phase = 'input';
    }
  }

  /** «Совпадает»: the account goes after the lock's password. */
  async approve(password: string | null): Promise<void> {
    this.error = null;
    this.phase = 'sending';
    try {
      await linkApi.approve(password);
      this.phase = 'sent';
    } catch (e) {
      this.error = asCloudError(e);
      this.phase = 'compare';
    }
  }

  /** «Не совпадает»: nothing goes, the link ends. */
  async mismatch(): Promise<void> {
    await this.end();
    this.phase = 'mismatch';
  }

  /**
   * Back to the field of the words. Once the account went, the wrap stays
   * on the relays until the new device says it is in or ten minutes pass:
   * «Назад» is not «Не совпадает».
   */
  async reset(): Promise<void> {
    if (this.phase === 'sent') {
      this.target = null;
      this.error = null;
    } else {
      await this.end();
    }
    this.phase = 'input';
  }

  private async end(): Promise<void> {
    try {
      await linkApi.cancel();
    } catch {
      // Nothing was sent, or the wrap expires by itself.
    }
    this.target = null;
    this.error = null;
  }
}
