// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { beforeEach, describe, expect, it } from 'vitest';
import { mockCommand } from '$lib/core/api';
import { asCloudError, IDLE_ACTIVATION, type CloudAccount, type CloudStatus } from '$lib/cloud/api';
import { activating, cloudPage, errorLine, failedStatus, statusLine } from '$lib/cloud/status.svelte';
import { emptyDeleteForm, viewOf } from '$lib/cloud/account';

const words = { notConnected: 'Not connected on this device.', name: 'Veydan Cloud' };

const account: CloudAccount = {
  npub: '3f'.repeat(32),
  state: 'free',
  plan: 'free',
  until: null,
  limits: { 'vault.files_bytes': 2147483648 },
  usage: { as_of: 1, vault: { files_bytes: 1024, text_bytes: 10 }, media: null },
};

const unconfigured: CloudStatus = { configured: false, account: null, plan: null };
const withAccount: CloudStatus = {
  configured: true,
  account,
  plan: 'free',
  servers: 'veydan',
  kept: null,
  error: null,
  activation: IDLE_ACTIVATION,
};

describe('the line of the cloud page', () => {
  it('says not connected until a host answered, then the plan', () => {
    expect(statusLine(null, words)).toBe('');
    expect(statusLine(unconfigured, words)).toBe(words.notConnected);
    expect(statusLine({ configured: true, account, plan: 'free' }, words)).toBe('Veydan Cloud · free');
  });

  it('reads any other rejection as an error of no known word', () => {
    expect(asCloudError(new Error('boom'))).toEqual({ code: 'unknown', message: 'boom', request_id: null });
    expect(asCloudError({ code: 'transport', message: 'down' })).toEqual({
      code: 'transport',
      message: 'down',
      request_id: null,
    });
    expect(errorLine(null)).toBe('');
  });

  it('knows an activation under way', () => {
    expect(activating(IDLE_ACTIVATION)).toBe(false);
    expect(activating({ ...IDLE_ACTIVATION, phase: 'solving' })).toBe(true);
    expect(activating({ ...IDLE_ACTIVATION, phase: 'done' })).toBe(false);
  });
});

describe('the store of the cloud page', () => {
  beforeEach(() => {
    mockCommand('cloud_status', withAccount);
    cloudPage.error = null;
    cloudPage.deleted = null;
  });

  it('reads cloud_status and the activation it carries', async () => {
    mockCommand('cloud_status', { ...withAccount, activation: { ...IDLE_ACTIVATION, phase: 'solving', expected: 128 } });
    await cloudPage.load();
    expect(cloudPage.status?.account?.plan).toBe('free');
    expect(cloudPage.activation.phase).toBe('solving');
    expect(cloudPage.error).toBeNull();
  });

  it('keeps a failure as the error of the seams, to branch on its code', async () => {
    const refused = { code: 'not_activated', message: 'no account', request_id: '9c' };
    mockCommand('cloud_status', () => {
      throw refused;
    });
    await cloudPage.load();
    expect(cloudPage.status?.error).toEqual(refused);
    expect(errorLine(cloudPage.status?.error ?? null)).toBe('not_activated: no account');
    mockCommand('cloud_status', unconfigured);
    await cloudPage.load();
    expect(cloudPage.status?.error).toBeUndefined();
    expect(cloudPage.error).toBeNull();
  });

  it('leaves «loading» when cloud_status itself fails, for the words and a retry', async () => {
    const stopped = { code: 'messenger_stopped', message: 'the messenger is off', request_id: null };
    mockCommand('cloud_status', () => {
      throw stopped;
    });
    await cloudPage.load();
    expect(viewOf(cloudPage.status)).toBe('unreachable');
    expect(cloudPage.status?.error?.code).toBe('messenger_stopped');
    expect(cloudPage.error).toBeNull();
    // Retry: the page reads the status again and shows it.
    mockCommand('cloud_status', withAccount);
    await cloudPage.load();
    expect(viewOf(cloudPage.status)).toBe('account');
  });

  it('keeps what the device kept when cloud_status fails after', () => {
    const kept = { ...withAccount, account: null, kept: { npub: '3f'.repeat(32), state: 'free' as const, plan: 'free', until: null, grace_until: null, limits: {}, usage: null, last_seen_at: null, refreshed_at: 1 } };
    const failed = failedStatus({ code: 'no_identity', message: '', request_id: null }, kept);
    expect(failed.kept?.plan).toBe('free');
    expect(failed.configured).toBe(false);
    expect(failedStatus({ code: 'x', message: '', request_id: null }, null).kept).toBeNull();
  });

  it('starts the free plan and keeps where it is', async () => {
    mockCommand('cloud_activate_free', { phase: 'asking', attempts: 0, expected: 0, error: null });
    await cloudPage.activate();
    expect(cloudPage.activation.phase).toBe('asking');
    expect(cloudPage.busy).toBe(false);
  });

  it('redeems a key and reads the page again; a refusal keeps its word', async () => {
    let asked: unknown = null;
    mockCommand('cloud_redeem', (args: { key: string }) => {
      asked = args.key;
      return { ...account, plan: 'plus', state: 'paid' };
    });
    expect(await cloudPage.redeem('7K2M-ABCD-EFGH-JKMN-PQRS')).toBe(true);
    expect(asked).toBe('7K2M-ABCD-EFGH-JKMN-PQRS');
    mockCommand('cloud_redeem', () => {
      throw { code: 'key_used', message: 'used', request_id: null };
    });
    expect(await cloudPage.redeem('7K2M-ABCD-EFGH-JKMN-PQRS')).toBe(false);
    expect(cloudPage.error?.code).toBe('key_used');
  });

  it('deletes the account with the word, the lock and the choices, and keeps the receipt', async () => {
    let sent: Record<string, unknown> | null = null;
    let wiped = false;
    mockCommand('cloud_delete_account', (args: Record<string, unknown>) => {
      sent = args;
      return { deleted_at: 1791012345, receipt: '7K2M-ABCD-EFGH', shares_revoked: 0, from_card: false };
    });
    mockCommand('messenger_identity_delete', () => {
      wiped = true;
    });
    mockCommand('sync_get_config', { backend: 'folder' });
    const form = { ...emptyDeleteForm(true), word: 'DELETE', secret: '4821', wipeDevice: true };
    const receipt = await cloudPage.deleteAccount(form);
    expect(receipt?.receipt).toBe('7K2M-ABCD-EFGH');
    expect(cloudPage.deleted?.receipt).toBe('7K2M-ABCD-EFGH');
    expect(sent).toEqual({ confirm: 'DELETE', lockSecret: '4821', revokeShares: true });
    expect(wiped).toBe(true);
  });

  it('drops the receipt once the identity has a new account', async () => {
    mockCommand('cloud_status', { ...withAccount, account: null, plan: null });
    await cloudPage.load();
    cloudPage.deleted = { deleted_at: 1, receipt: '7K2M-ABCD-EFGH', shares_revoked: 0, from_card: false };
    await cloudPage.load();
    expect(cloudPage.deleted?.receipt).toBe('7K2M-ABCD-EFGH');
    mockCommand('cloud_status', withAccount);
    await cloudPage.load();
    expect(cloudPage.deleted).toBeNull();
  });

  it('leaves the sync of the cloud after a deletion, and only that sync', async () => {
    let left = 0;
    mockCommand('cloud_delete_account', { deleted_at: 1, receipt: '7K2M-ABCD-EFGH', shares_revoked: 0, from_card: false });
    mockCommand('sync_get_config', { backend: 'cloud' });
    mockCommand('sync_status', { joined: true });
    mockCommand('sync_leave', () => {
      left++;
      return { joined: false };
    });
    await cloudPage.deleteAccount({ ...emptyDeleteForm(false), word: 'DELETE' });
    expect(left).toBe(1);
    mockCommand('sync_get_config', { backend: 's3' });
    await cloudPage.deleteAccount({ ...emptyDeleteForm(false), word: 'DELETE' });
    expect(left).toBe(1);
  });

  it('keeps the word of a refused deletion', async () => {
    mockCommand('cloud_delete_account', () => {
      throw { code: 'lock_wrong', message: 'wrong', request_id: null };
    });
    const receipt = await cloudPage.deleteAccount({ ...emptyDeleteForm(true), word: 'DELETE', secret: 'x' });
    expect(receipt).toBeNull();
    expect(cloudPage.error?.code).toBe('lock_wrong');
    expect(cloudPage.deleted).toBeNull();
  });
});
