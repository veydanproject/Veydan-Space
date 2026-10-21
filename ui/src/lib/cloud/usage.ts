// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// `cloud_usage` (internal/vcloud-plan.md 4.7): the pools of the plan, how
// full they are and what takes the room, for the banner of a full pool.
// The levels are the core's, which the sync settings read too.

import { call, mockCommand } from '$lib/core/api';
import { FULL_PERCENT, WARN_PERCENT, type CloudPoolView, type CloudUsageView } from '$lib/core/sync/cloud';

export { fullestPool } from '$lib/core/sync/cloud';

/** What takes the room, by the paths of the vault. */
export interface CloudCategory {
  id: 'journal' | 'files' | 'large_files' | 'recovery' | 'other';
  pool: string;
  bytes: number;
  objects: number;
}

/**
 * What the host itself wrote (`GET /vault/v1/usage`, spec/vstore.md §6), of
 * which the page reads the media and the shared storage; an older host
 * sends neither.
 */
export interface CloudHostUsage {
  media?: { used: number; limit: number | null; known: boolean; reconciled_at: number | null } | null;
  total?: { used: number; limit: number | null } | null;
}

/** The answer of `cloud_usage`. */
export interface CloudUsage extends CloudUsageView {
  usage?: CloudHostUsage;
  pools: CloudPoolView[];
  categories: CloudCategory[];
  /** False: notes' blobs and profiles' files are not told apart yet. */
  by_owner: boolean;
}

export const usageApi = {
  usage: () => call<CloudUsage>('cloud_usage'),
};

/** What the banner says: nothing, almost full, or full. Only for a device that syncs through the cloud. */
export function bannerOf(usage: CloudUsage | null): 'none' | 'warn' | 'full' {
  if (!usage?.on_cloud) return 'none';
  if (usage.level >= FULL_PERCENT) return 'full';
  if (usage.level >= WARN_PERCENT) return 'warn';
  return 'none';
}

/** The categories with room in them, the largest first. */
export function largest(usage: CloudUsage | null): CloudCategory[] {
  return (usage?.categories ?? []).filter((c) => c.bytes > 0).sort((a, b) => b.bytes - a.bytes);
}

// In a plain browser no host is known.
mockCommand('cloud_usage', () => {
  throw { code: 'no_host', message: 'no host of Veydan Cloud is known on this device', request_id: null };
});
