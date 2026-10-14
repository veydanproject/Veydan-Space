// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { ERR_JOIN_OWN_LOCK, ERR_NO_ATOMIC_MOVE, ERR_NO_CONDITIONAL_PUT, ERR_SYNC_RUNNING, ERR_VAULT_UNREAD, joinErrorKey, storageErrorKey } from './join-error';

describe('joinErrorKey', () => {
  it('knows the text the lock crate refuses a join with', () => {
    const source = readFileSync(new URL('../../../../crates/lock/src/vault.rs', import.meta.url), 'utf8');
    expect(source).toContain(`pub const ERR_JOIN_OWN_LOCK: &str = ${JSON.stringify(ERR_JOIN_OWN_LOCK)};`);
  });

  it('translates only the refusal', () => {
    expect(joinErrorKey({ code: 'other', message: ERR_JOIN_OWN_LOCK })).toBe('settings_sync_join_own_lock');
    expect(joinErrorKey({ code: 'other', message: 'wrong passphrase' })).toBeNull();
    expect(joinErrorKey({ code: 'db', message: ERR_JOIN_OWN_LOCK })).toBeNull();
    expect(joinErrorKey('boom')).toBeNull();
    expect(joinErrorKey(null)).toBeNull();
  });

  it('gives the words of the wait when the storage of Veydan Cloud does not know the account yet', () => {
    const refusal = 'auth: get: 403 Forbidden no_account: the signer has no account of Veydan Cloud (request 0bc5)';
    expect(joinErrorKey({ code: 'other', message: refusal })).toBe('settings_sync_cloud_not_ready');
    expect(joinErrorKey({ code: 'other', message: 'auth: get: 403 Forbidden suspended' })).toBeNull();
  });

  it('says in words that the storage cannot make a write atomic or conditional', () => {
    expect(joinErrorKey({ code: 'other', message: `storage: ${ERR_NO_ATOMIC_MOVE}` })).toBe('settings_sync_storage_no_move');
    expect(joinErrorKey({ code: 'other', message: `storage: ${ERR_NO_CONDITIONAL_PUT}` })).toBe('settings_sync_storage_no_conditional');
    expect(joinErrorKey({ code: 'db', message: `storage: ${ERR_NO_ATOMIC_MOVE}` })).toBeNull();
  });

  it('gives the words of the wait when the vault does not read in full yet', () => {
    const source = readFileSync(new URL('../../../../crates/sync-host/src/join.rs', import.meta.url), 'utf8');
    expect(source).toContain(`pub const ERR_VAULT_UNREAD: &str = ${JSON.stringify(ERR_VAULT_UNREAD)};`);
    const refusal = `${ERR_VAULT_UNREAD}: storage: chunk devices/a1/log/00000003.bin listed but not readable`;
    expect(joinErrorKey({ code: 'other', message: refusal })).toBe('settings_sync_join_unread');
    // A restore of Veydan Cloud carries it as an error of the storage.
    expect(joinErrorKey({ code: 'storage', message: refusal })).toBe('settings_sync_join_unread');
    expect(joinErrorKey({ code: 'db', message: refusal })).toBeNull();
  });

  it('gives the words of the wait while a cycle holds the binding', () => {
    const source = readFileSync(new URL('../../../../crates/sync-host/src/join.rs', import.meta.url), 'utf8');
    expect(source).toContain(`pub const ERR_SYNC_RUNNING: &str = ${JSON.stringify(ERR_SYNC_RUNNING)};`);
    expect(joinErrorKey({ code: 'other', message: ERR_SYNC_RUNNING })).toBe('settings_sync_busy');
    expect(joinErrorKey({ code: 'db', message: ERR_SYNC_RUNNING })).toBeNull();
  });
});

describe('storageErrorKey', () => {
  it('knows the texts the sync crate refuses a storage with', () => {
    const source = readFileSync(new URL('../../../../crates/sync/src/storage.rs', import.meta.url), 'utf8').replace(/\s+/g, ' ');
    expect(source).toContain(`pub const ERR_NO_ATOMIC_MOVE: &str = ${JSON.stringify(ERR_NO_ATOMIC_MOVE)};`);
    expect(source).toContain(`pub const ERR_NO_CONDITIONAL_PUT: &str = ${JSON.stringify(ERR_NO_CONDITIONAL_PUT)};`);
  });

  it('reads the last error of the cycle', () => {
    expect(storageErrorKey(`storage: ${ERR_NO_CONDITIONAL_PUT}`)).toBe('settings_sync_storage_no_conditional');
    expect(storageErrorKey('storage: webdav put: 507 Insufficient Storage')).toBeNull();
    expect(storageErrorKey(null)).toBeNull();
    expect(storageErrorKey('')).toBeNull();
  });
});
