// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// «The identity has an account» before sync is turned on at the first
// start: nothing is asked where there is one, the activation of the free
// plan is followed to its end where there is none.

import { describe, expect, it, vi } from 'vitest';
import type { CloudActivation, CloudStatus } from '$lib/cloud/api';
import { ensureAccount, type EnsureDeps } from '$lib/cloud/ensure';

const none: CloudStatus = { configured: true, account: null, plan: null, servers: 'veydan', kept: null };
const phase = (p: CloudActivation['phase'], error: CloudActivation['error'] = null): CloudActivation => ({ phase: p, attempts: 0, expected: 128, error });

function deps(over: Partial<EnsureDeps>): EnsureDeps {
  return {
    status: async () => none,
    activate: async () => phase('solving'),
    follow: async () => () => {},
    limitMs: 20,
    ...over,
  };
}

describe('the account of Veydan Cloud before sync is turned on', () => {
  it('is not activated twice', async () => {
    const activate = vi.fn(async () => phase('done'));
    await ensureAccount(deps({ status: async () => ({ ...none, plan: 'free', account: { plan: 'free' } as CloudStatus['account'] }), activate }));
    // The device keeps the account while the server is silent: that is an account too.
    await ensureAccount(deps({ status: async () => ({ ...none, configured: false, kept: { plan: 'plus' } as CloudStatus['kept'] }), activate }));
    expect(activate).not.toHaveBeenCalled();
  });

  it('is there when the activation answers that it ended at once', async () => {
    await expect(ensureAccount(deps({ activate: async () => phase('done') }))).resolves.toBeUndefined();
  });

  it('is waited for through the events of the activation', async () => {
    let tell: (a: CloudActivation) => void = () => {};
    const stop = vi.fn();
    const waiting = ensureAccount(
      deps({
        limitMs: 5_000,
        follow: async (on) => {
          tell = on;
          return stop;
        },
        activate: async () => {
          setTimeout(() => tell(phase('activating')), 1);
          setTimeout(() => tell(phase('done')), 5);
          return phase('solving');
        },
      }),
    );
    await expect(waiting).resolves.toBeUndefined();
    expect(stop).toHaveBeenCalledOnce();
  });

  it('refuses in the words of the server when the activation failed', async () => {
    const failed = phase('failed', { code: 'made_up_code', message: 'the host is full', request_id: null });
    await expect(ensureAccount(deps({ activate: async () => failed }))).rejects.toThrow('the host is full');
  });

  it('is there when a second activation is refused because the first made it', async () => {
    let asked = 0;
    const status = async () => (asked++ === 0 ? none : { ...none, plan: 'free', account: { plan: 'free' } as CloudStatus['account'] });
    const failed = phase('failed', { code: 'made_up_code', message: 'the account exists', request_id: null });
    await expect(ensureAccount(deps({ status, activate: async () => failed }))).resolves.toBeUndefined();
  });

  it('refuses when the activation does not end in time', async () => {
    await expect(ensureAccount(deps({}))).rejects.toThrow();
  });

  it('refuses in the words of the refusal when the status cannot be read', async () => {
    const status = async () => {
      throw { code: 'made_up_code', message: 'no identity', request_id: null };
    };
    await expect(ensureAccount(deps({ status }))).rejects.toThrow('no identity');
  });
});
