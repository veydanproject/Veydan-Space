// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// «The identity has an account» as one promise (vcloud-plan.md 4.3, 4.5):
// the first start turns on sync through Veydan Cloud by one button, and the
// free plan may still be on its way then — started by the choice of the
// servers a few seconds before, or not started at all. The activation is
// started where there is no account and followed to its end by its events;
// a refusal comes back in the words of the module.

import { get } from 'svelte/store';
import { t } from '$lib/core/i18n';
import { asCloudError, cloudApi, type CloudActivation, type CloudError } from '$lib/cloud/api';
import { errorKeyOf } from '$lib/cloud/account';

const ACTIVATION_EVENT = 'cloud://activation';
/** The proof of work takes seconds; a phone of 2019 up to fifteen (cloud-recovery.md §13). */
const LIMIT_MS = 120_000;

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

const ended = (a: CloudActivation | null | undefined) => a?.phase === 'done' || a?.phase === 'failed';

/** The words of a refusal: the module's own by its code, else what the server said. */
function words(e: CloudError | null): string {
  const key = e ? errorKeyOf(e.code) : null;
  if (key) return get(t)(key);
  return e?.message || e?.code || get(t)('cloud_ensure_timeout');
}

/** What the activation needs of the outside; the tests give their own. */
export interface EnsureDeps {
  status: typeof cloudApi.status;
  activate: typeof cloudApi.activateFree;
  /** Follows the events of the activation; returns the way to stop. */
  follow: (on: (a: CloudActivation) => void) => Promise<() => void>;
  limitMs: number;
}

async function followEvents(on: (a: CloudActivation) => void): Promise<() => void> {
  if (!isTauri) return () => {};
  const { listen } = await import('@tauri-apps/api/event');
  return listen<CloudActivation>(ACTIVATION_EVENT, (e) => on(e.payload));
}

const real: EnsureDeps = { status: cloudApi.status, activate: cloudApi.activateFree, follow: followEvents, limitMs: LIMIT_MS };

/** Resolves when the identity has an account; throws an `Error` with the words to show when it cannot have one. */
export async function ensureAccount(deps: EnsureDeps = real): Promise<void> {
  let status;
  try {
    status = await deps.status();
  } catch (e) {
    throw new Error(words(asCloudError(e)));
  }
  if (status.account || status.kept) return;

  let last: CloudActivation | null = null;
  let wake: (() => void) | null = null;
  // The events are followed before the activation is asked for: one that ends at once is not missed.
  const stop = await deps.follow((a) => {
    last = a;
    if (ended(a)) wake?.();
  });
  try {
    try {
      const started = await deps.activate();
      if (!ended(last)) last = started;
    } catch (e) {
      throw new Error(words(asCloudError(e)));
    }
    if (!ended(last)) {
      await new Promise<void>((resolve) => {
        wake = resolve;
        setTimeout(resolve, deps.limitMs);
      });
    }
    const end = last as CloudActivation | null;
    if (end?.phase === 'done') return;
    // An activation that ended a moment before this one began made the account: the second one is refused, and the account is there.
    const now = await deps.status().catch(() => null);
    if (now?.account) return;
    throw new Error(words(end?.phase === 'failed' ? end.error : null));
  } finally {
    stop();
  }
}
