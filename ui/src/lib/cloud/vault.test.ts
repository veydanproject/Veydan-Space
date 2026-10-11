// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { SyncConfig } from '$lib/core/api';
import { vaultState } from '$lib/cloud/vault';

const config = (c: Partial<SyncConfig>): SyncConfig =>
  ({
    enabled: false,
    backend: 'folder',
    folder_path: '',
    s3: { endpoint: '', region: '', bucket: '', prefix: '', access_key: '', has_secret_key: false, path_style: false },
    webdav: { url: '', username: '', has_password: false },
    interval_sec: 300,
    ...c,
  }) as SyncConfig;

describe('the vault of sync before the recovery is set up', () => {
  it('is the one the device syncs with, whatever the form says', () => {
    expect(vaultState({ joined: true }, config({}))).toBe('joined');
    expect(vaultState({ joined: true }, null)).toBe('joined');
  });

  it('is none on a device that syncs nowhere: the setup turns on Veydan Cloud', () => {
    expect(vaultState(null, null)).toBe('none');
    expect(vaultState({ joined: false }, config({}))).toBe('none');
    expect(vaultState({ joined: false }, config({ backend: 's3' }))).toBe('none');
    expect(vaultState({ joined: false }, config({ backend: 'webdav' }))).toBe('none');
    expect(vaultState({ joined: false }, config({ backend: 'cloud' }))).toBe('none');
  });

  it('is the user\'s own once a storage is chosen: it is never replaced', () => {
    expect(vaultState({ joined: false }, config({ folder_path: '/home/anna/Sync' }))).toBe('own');
    const s3 = config({}).s3;
    expect(vaultState({ joined: false }, config({ backend: 's3', s3: { ...s3, bucket: 'notes' } }))).toBe('own');
    const webdav = config({}).webdav;
    expect(vaultState({ joined: false }, config({ backend: 'webdav', webdav: { ...webdav, url: 'https://dav.example.com' } }))).toBe('own');
  });
});
