// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Commands of the Cloud module (internal/vcloud-plan.md 2.2, 4.3-4.6): the
// account of Veydan Cloud on this device, its activation, a key, the
// deletion of the account.

import { call, mockCommand } from '$lib/core/api';

/** The state of an account (services/cloud/spec/cloud-api.md 2.4). */
export type CloudAccountState = 'free' | 'paid' | 'grace' | 'suspended';

/** Usage as the account reports it (cloud-api.md 4.3). */
export interface CloudAccountUsage {
  as_of: number | null;
  vault: { files_bytes: number; text_bytes: number } | null;
  media: { upload_today_bytes: number; storage_bytes: number } | null;
}

/** The numbers of the recovery config (cloud-api.md 4.3). */
export interface CloudRecoveryInfo {
  configured: boolean;
  k: number | null;
  n: number | null;
  generation: number | null;
}

/** The plan that starts when the current one ends (cloud-api.md 4.3 `next`). */
export interface CloudNextPlan {
  plan: string;
  since: number;
  until: number | null;
}

/** `GET /cloud/v1/account` (cloud-api.md 4.3); `subscription` only after a key (4.2). */
export interface CloudAccount {
  npub: string;
  state: CloudAccountState;
  plan: string;
  until: number | null;
  grace_until?: number | null;
  /** A plan bought while another runs, from the day that one ends (cloud-api.md 4.3). */
  next?: CloudNextPlan | null;
  /** The effective limits by the keys of the contract (`vault.files_bytes`, …). */
  limits?: Record<string, number>;
  usage?: CloudAccountUsage | null;
  recovery?: CloudRecoveryInfo | null;
  last_seen_at?: number | null;
  server_time?: number | null;
  subscription?: { plan: string; since: number; until: number | null; source: string } | null;
}

/** What this device keeps of the account (the box `cloud` of the lock). */
export interface CloudKeptAccount {
  npub: string;
  state: CloudAccountState;
  plan: string;
  until: number | null;
  grace_until: number | null;
  limits: Record<string, number>;
  usage: CloudAccountUsage | null;
  last_seen_at: number | null;
  /** When this device last heard the account from the server. */
  refreshed_at: number;
}

/** A price of a plan (cloud-api.md 4.5): the amount in the minor units of the currency, for `period_days`. */
export interface CloudPlanPrice {
  currency: string;
  amount_minor: number;
  period_days: number;
}

/** A plan a person can choose (cloud-api.md 4.5): its effective limits and what it costs. */
export interface CloudPlanOffer {
  id: string;
  name: string;
  /** The plan every account starts with. */
  free: boolean;
  period_days: number;
  sort: number;
  /** The same keys and numbers an account of the plan carries (`vault.files_bytes`, …). */
  limits: Record<string, number>;
  prices: CloudPlanPrice[];
}

/** `GET /cloud/v1/plans` (cloud-api.md 4.5): the plans on offer, in the order of the owner. */
export interface CloudPlans {
  plans: CloudPlanOffer[];
  server_time?: number | null;
}

/** Whose servers the messenger uses; Veydan Cloud is only with `veydan`. */
export type CloudServers = 'none' | 'unchosen' | 'own' | 'veydan';

export type CloudPhase = 'idle' | 'asking' | 'solving' | 'activating' | 'done' | 'failed';

/** Where an activation of the free plan is (event `cloud://activation`). */
export interface CloudActivation {
  phase: CloudPhase;
  attempts: number;
  /** About how many attempts the work takes: 2^d. */
  expected: number;
  error: CloudError | null;
}

/** The answer of `cloud_status`. */
export interface CloudStatus {
  /** A host of Veydan Cloud answered for the identity (with an account or without). */
  configured: boolean;
  account: CloudAccount | null;
  plan: string | null;
  servers?: CloudServers;
  /** What this device keeps, shown when the server did not answer. */
  kept?: CloudKeptAccount | null;
  /** Why the server did not answer. */
  error?: CloudError | null;
  activation?: CloudActivation;
}

/** The answer of `cloud_delete_account`, and the payload of `cloud://account-deleted`. */
export interface CloudDeleteReceipt {
  deleted_at: number;
  /** The number for the stores: 12 Crockford characters in three groups of four, 14 with the dashes (`7K2M-ABCD-EFGH`). */
  receipt: string;
  shares_revoked: number;
  /** Learnt from the card of the bot: another device deleted it. */
  from_card: boolean;
}

/** The answer of `cloud_limits`: the plan and limits kept on this device. */
export interface CloudLimits {
  plan: string | null;
  state: CloudAccountState | null;
  until: number | null;
  limits: Record<string, number>;
  refreshed_at: number | null;
}

/**
 * What a command of the module fails with (cloud-api.md §12): `code` is a
 * word of section 9 (`not_activated`, `auth_expired`, `suspended`, …), of
 * the client (`transport`, `no_host`, …) or of the module (`lock_wrong`);
 * a screen branches on it alone, `message` is for a person.
 */
export interface CloudError {
  code: string;
  message: string;
  request_id: string | null;
}

/** Whether a rejection of a command is a `CloudError`. */
export function isCloudError(e: unknown): e is CloudError {
  return (
    typeof e === 'object' &&
    e !== null &&
    typeof (e as CloudError).code === 'string' &&
    typeof (e as CloudError).message === 'string'
  );
}

/** The storage of the vault an escrow or a link names is not within reach of this device (cloud-recovery.md §7). */
export const VAULT_UNREACHABLE = 'vault_unreachable';

/** Any rejection as a `CloudError`; one that is not keeps its text under `code` `unknown`. */
export function asCloudError(e: unknown): CloudError {
  if (isCloudError(e)) return { code: e.code, message: e.message, request_id: e.request_id ?? null };
  return { code: 'unknown', message: e instanceof Error ? e.message : String(e), request_id: null };
}

/** The event of the progress of an activation, with a `CloudActivation`. */
export const ACTIVATION_EVENT = 'cloud://activation';
/** The event of an account gone, with a `CloudDeleteReceipt`. */
export const DELETED_EVENT = 'cloud://account-deleted';

/** What the deletion of the account sends (vcloud-plan.md 4.6). */
export interface CloudDeleteRequest {
  /** The word DELETE as typed. */
  confirm: string;
  /** The secret of the lock, when one is set. */
  lockSecret: string | null;
  /** Ask the trusted people to drop their shares first. */
  revokeShares: boolean;
}

/** The Recovery Key as `cloud_recovery_key_show_once` and `…_reissue` hand it out, once. */
export interface CloudRecoveryKeyIssued {
  /** The 12 words, one space between. */
  words: string;
  generation: number;
}

/** The answer of `cloud_recovery_status`: what this device knows of the Recovery Key. */
export interface CloudRecoveryKeyStatus {
  /** A key of the cloud is issued on this device and the escrow is written. */
  issued: boolean;
  generation: number;
  /** Unix seconds of the last write of the escrow by this device. */
  escrow_written_at: number | null;
  /** The recovery wrap of the lock: 1 a code, 2 the words, null without. */
  lock_recovery_version: 1 | 2 | null;
}

/** Where a restore from the 12 words stands (event `cloud://restore`). */
export type CloudRestorePhase = 'reading' | 'opening' | 'vault' | 'key_row' | 'lock' | 'identity' | 'done';

/** The event of the progress of a restore, with `{ phase: CloudRestorePhase }`. */
export const RESTORE_EVENT = 'cloud://restore';

/** The answer of `cloud_restore_with_words` (cloud-recovery.md §7). */
export interface CloudRestored {
  /** The owner, 64 hex. */
  npub: string;
  /** `kept`: the lock of the account took the new password; `reinstalled`: set up anew; `no_lock`. */
  lock: 'kept' | 'reinstalled' | 'no_lock';
  /** Whether the vault of sync was joined; false when the person went on without a storage out of reach. */
  vault_joined: boolean;
  k: number;
  n: number;
  generation: number;
  /** After `reinstalled`: whether the copy was written again with the key in use; `false` asks the user to repeat it. */
  escrow_refreshed: boolean | null;
}

/** The new secret of the lock a restore sets. */
export interface CloudRestoreSecret {
  password: string;
  kind: 'pin' | 'password';
  hint: string | null;
}

export const cloudApi = {
  status: () => call<CloudStatus>('cloud_status'),
  recoveryStatus: () => call<CloudRecoveryKeyStatus>('cloud_recovery_status'),
  /** The first setup of recovery: the words come once. `current` is the secret of the lock. `anyway` issues although the key was issued on another device (`issued_elsewhere`). */
  recoveryKeyShowOnce: (current: string | null, anyway = false) =>
    call<CloudRecoveryKeyIssued>('cloud_recovery_key_show_once', { current, anyway }),
  /** A new Recovery Key in place of the one in force, all or nothing. */
  recoveryKeyReissue: (current: string | null, anyway = false) =>
    call<CloudRecoveryKeyIssued>('cloud_recovery_key_reissue', { current, anyway }),
  /**
   * A clean install and the 12 words: the account restored in the order of §7.
   * `withoutVault` is the answer to `vault_unreachable`: the identity and the lock alone.
   */
  restoreWithWords: (words: string, secret: CloudRestoreSecret, withoutVault = false) =>
    call<CloudRestored>('cloud_restore_with_words', { words, ...secret, withoutVault }),
  /** Starts the activation of the free plan; answers at once with where it is. */
  activateFree: () => call<CloudActivation>('cloud_activate_free'),
  redeem: (key: string) => call<CloudAccount>('cloud_redeem', { key }),
  refresh: () => call<CloudAccount | null>('cloud_account_refresh'),
  limits: () => call<CloudLimits>('cloud_limits'),
  /** The plans on offer; a host that does not know the route fails, and the page shows no choice. */
  plans: () => call<CloudPlans>('cloud_plans'),
  deleteAccount: (r: CloudDeleteRequest) =>
    call<CloudDeleteReceipt>('cloud_delete_account', {
      confirm: r.confirm,
      lockSecret: r.lockSecret,
      revokeShares: r.revokeShares,
    }),
};

export const IDLE_ACTIVATION: CloudActivation = { phase: 'idle', attempts: 0, expected: 0, error: null };

// In a plain browser no host is known: the page shows what it shows on a
// product without the messenger's way to the project.
const noHost: CloudError = { code: 'no_host', message: 'no host of Veydan Cloud is known on this device', request_id: null };
mockCommand('cloud_status', {
  configured: false,
  account: null,
  plan: null,
  servers: 'none',
  kept: null,
  error: noHost,
  activation: IDLE_ACTIVATION,
} satisfies CloudStatus);
const refuse = () => {
  throw noHost;
};
mockCommand('cloud_activate_free', refuse);
mockCommand('cloud_redeem', refuse);
mockCommand('cloud_account_refresh', refuse);
mockCommand('cloud_delete_account', refuse);
mockCommand('cloud_plans', refuse);
mockCommand('cloud_limits', { plan: null, state: null, until: null, limits: {}, refreshed_at: null } satisfies CloudLimits);
mockCommand('cloud_recovery_status', {
  issued: false,
  generation: 0,
  escrow_written_at: null,
  lock_recovery_version: null,
} satisfies CloudRecoveryKeyStatus);
mockCommand('cloud_recovery_key_show_once', refuse);
mockCommand('cloud_recovery_key_reissue', refuse);
mockCommand('cloud_restore_with_words', refuse);

// ── Social Recovery (cloud-recovery.md §8, stage C5) ─────────────────────

/** An approved contact, as the picker of the trusted people shows it (`cloud_contacts_approved`). */
export interface CloudContact {
  pubkey: string;
  name: string;
}

/** The state of a share at one person (cloud-recovery.md §8.3). */
export type CloudTrusteeHealth = 'waiting' | 'not_delivered' | 'delivered' | 'alive' | 'unconfirmed';

/** One trusted person as the screen shows them. */
export interface CloudTrustee {
  pubkey: string;
  name: string;
  index: number;
  health: CloudTrusteeHealth;
  acked_at: number | null;
  /** The last pong with «have»: «проверено N дней назад». */
  checked_at: number | null;
}

/** The answer of `cloud_trustees_status`, and the payload of `cloud://trustees`. */
export interface CloudTrustees {
  configured: boolean;
  k: number;
  n: number;
  generation: number;
  people: CloudTrustee[];
  acked: number;
  alive: number;
  checked_at: number | null;
}

/** What «Подтвердить восстановление» shows (`cloud_help_lookup`). */
export interface CloudHelpView {
  npub: string;
  name: string | null;
  request_id: string;
  /** The code computed here from the request: `AB7K-2QZD-9MXP`. */
  code: string;
  state: 'open' | 'cancelled' | 'expired' | 'closed' | string;
  created_at: number;
  expires_at: number;
  closed_at: number | null;
  generation: number;
  /** Whether the code of the link is the code of the request; null without a link. */
  code_matches: boolean | null;
  can_confirm: boolean;
}

/** The share went to the one who asked (`cloud_help_confirm`). */
export interface CloudHelpSent {
  request_id: string;
  expires_at: number;
}

export type CloudRequestPhase = 'idle' | 'preparing' | 'waiting' | 'collected' | 'restored' | 'expired' | 'failed';

/** One person who confirmed the request. */
export interface CloudConfirmation {
  npub: string;
  index: number;
  /** Their share did not match the key: «доля повреждена». */
  forged: boolean;
}

/** What «Запрос отправлен» shows (`cloud_recover_start`, `cloud_recover_poll`). */
export interface CloudRequestView {
  phase: CloudRequestPhase;
  npub: string | null;
  request_id: string | null;
  code: string | null;
  link: string | null;
  k: number;
  n: number;
  created_at: number | null;
  expires_at: number | null;
  confirmed: CloudConfirmation[];
  attempts: number;
  error: CloudError | null;
}

/** The answer of `cloud_recover_finish`: the account restored, the words once more. */
export interface CloudRecovered {
  restored: CloudRestored;
  words: string;
  closed: boolean;
}

/** An alert from a card of the vcloud bot (`cloud_recovery_alerts`, `cloud://recovery-alerts`). */
export interface CloudAlert {
  id: string;
  t: 'recovery.requested' | 'recovery.escrow_read' | 'recovery.restored' | string;
  request_id: string | null;
  created_at: number | null;
  expires_at: number | null;
  at: number;
}

/** The answer of `cloud_recovery_cancel` (cloud-api.md 5.4). */
export interface CloudCancelled {
  state: string;
  frozen_until: number | null;
}

/** The event with the trusted people, a `CloudTrustees`. */
export const TRUSTEES_EVENT = 'cloud://trustees';
/** The event with the alerts of recovery, a `CloudAlert[]`. */
export const ALERTS_EVENT = 'cloud://recovery-alerts';

export const socialApi = {
  trustees: () => call<CloudTrustees>('cloud_trustees_status'),
  approved: () => call<CloudContact[]>('cloud_contacts_approved'),
  setup: (k: number, people: CloudContact[]) => call<CloudTrustees>('cloud_trustees_setup', { k, people }),
  disable: () => call<number>('cloud_trustees_disable'),
  check: () => call<CloudTrustees>('cloud_trustees_check'),
  helpLookup: (input: string) => call<CloudHelpView>('cloud_help_lookup', { input }),
  helpConfirm: (spoke: boolean) => call<CloudHelpSent>('cloud_help_confirm', { spoke }),
  helpClose: () => call<void>('cloud_help_close'),
  recoverStart: (npub: string) => call<CloudRequestView>('cloud_recover_start', { npub }),
  recoverPoll: () => call<CloudRequestView>('cloud_recover_poll'),
  recoverFinish: (secret: CloudRestoreSecret, withoutVault = false) =>
    call<CloudRecovered>('cloud_recover_finish', { ...secret, withoutVault }),
  recoverCancel: () => call<void>('cloud_recover_cancel'),
  alerts: () => call<CloudAlert[]>('cloud_recovery_alerts'),
  cancelRequest: (requestId: string) => call<CloudCancelled>('cloud_recovery_cancel', { requestId }),
  dismissAlert: (id: string) => call<void>('cloud_recovery_alert_dismiss', { id }),
};

export const NO_TRUSTEES: CloudTrustees = { configured: false, k: 3, n: 5, generation: 0, people: [], acked: 0, alive: 0, checked_at: null };
export const IDLE_REQUEST: CloudRequestView = {
  phase: 'idle', npub: null, request_id: null, code: null, link: null, k: 0, n: 0,
  created_at: null, expires_at: null, confirmed: [], attempts: 0, error: null,
};
mockCommand('cloud_trustees_status', NO_TRUSTEES);
mockCommand('cloud_contacts_approved', []);
mockCommand('cloud_trustees_setup', refuse);
mockCommand('cloud_trustees_disable', refuse);
mockCommand('cloud_trustees_check', NO_TRUSTEES);
mockCommand('cloud_help_lookup', refuse);
mockCommand('cloud_help_confirm', refuse);
mockCommand('cloud_help_close', undefined);
mockCommand('cloud_recover_start', refuse);
mockCommand('cloud_recover_poll', IDLE_REQUEST);
mockCommand('cloud_recover_finish', refuse);
mockCommand('cloud_recover_cancel', undefined);
mockCommand('cloud_recovery_alerts', []);
mockCommand('cloud_recovery_cancel', refuse);
mockCommand('cloud_recovery_alert_dismiss', undefined);
