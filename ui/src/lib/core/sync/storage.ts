// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import type { SyncConfig } from '$lib/core/api';

/**
 * Where the vault of a config lives, as one string: two configs with the
 * same answer reach the same storage. The keys, the region and the rest of
 * the form are how it is reached, not where it is.
 */
export function storageOf(cfg: SyncConfig): string {
  switch (cfg.backend) {
    case 'folder':
      return `folder ${cfg.folder_path.trim()}`;
    case 's3':
      return ['s3', cfg.s3.endpoint.trim(), cfg.s3.bucket.trim(), cfg.s3.prefix.trim()].join(' ');
    case 'webdav':
      return `webdav ${cfg.webdav.url.trim()}`;
    default:
      return cfg.backend;
  }
}
