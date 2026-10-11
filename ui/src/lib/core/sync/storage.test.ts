// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import type { SyncConfig } from '$lib/core/api';
import { storageOf } from '$lib/core/sync/storage';

const base: SyncConfig = {
  enabled: true,
  backend: 's3',
  folder_path: '/home/me/vault',
  s3: { endpoint: 'https://s3.example.com', region: 'eu', bucket: 'mine', prefix: 'veydan', access_key: 'AK', has_secret_key: true, path_style: true },
  webdav: { url: 'https://dav.example.com/veydan', username: 'me', has_password: true },
  interval_sec: 60,
  profile_files: true,
  device_name: 'desk',
  large_files: { chunk_mib: 8, parallelism: 3, resume: true },
};

describe('storageOf', () => {
  it('is another storage when the backend changes', () => {
    expect(storageOf({ ...base, backend: 'cloud' })).not.toBe(storageOf(base));
    expect(storageOf({ ...base, backend: 'folder' })).not.toBe(storageOf(base));
    expect(storageOf({ ...base, backend: 'webdav' })).not.toBe(storageOf(base));
  });

  it('is another storage when the place of the backend in use changes', () => {
    expect(storageOf({ ...base, s3: { ...base.s3, bucket: 'other' } })).not.toBe(storageOf(base));
    expect(storageOf({ ...base, s3: { ...base.s3, prefix: '' } })).not.toBe(storageOf(base));
    expect(storageOf({ ...base, s3: { ...base.s3, endpoint: 'https://s3.other.example' } })).not.toBe(storageOf(base));
    const dav = { ...base, backend: 'webdav' };
    expect(storageOf({ ...dav, webdav: { ...dav.webdav, url: 'https://dav.example.com/other' } })).not.toBe(storageOf(dav));
    const folder = { ...base, backend: 'folder' };
    expect(storageOf({ ...folder, folder_path: '/mnt/other' })).not.toBe(storageOf(folder));
  });

  it('is the same storage when only the way to it or the rest of the form changes', () => {
    expect(storageOf({ ...base, s3: { ...base.s3, access_key: 'AK2', secret_key: 'new', region: 'us', path_style: false } })).toBe(storageOf(base));
    expect(storageOf({ ...base, interval_sec: 300, enabled: false, device_name: 'laptop' })).toBe(storageOf(base));
    // The fields of a backend that is not in use are not where the vault is.
    expect(storageOf({ ...base, folder_path: '/mnt/other', webdav: { ...base.webdav, url: '' } })).toBe(storageOf(base));
    expect(storageOf({ ...base, s3: { ...base.s3, bucket: ' mine ' } })).toBe(storageOf(base));
  });
});
