// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { afterEach, describe, expect, it, vi } from 'vitest';
import { call, mockCommand, type SyncConfig } from '$lib/core/api';
import { registry } from '$lib/core/registry';
import {
  asksForProfileFiles,
  cloudChoice,
  cloudNotReady,
  cloudVaultExists,
  connectCloudVault,
  filesLimit,
  fullestPool,
  loadCloud,
  planOf,
  setCloudSource,
  usageTone,
  type CloudSource,
  type CloudStatusView,
  type CloudUsageView,
} from '$lib/core/sync/cloud';

/** What the module gives the core as it is loaded: its two commands. */
const ofTheModule: CloudSource = {
  status: () => call<CloudStatusView>('cloud_status'),
  usage: () => call<CloudUsageView>('cloud_usage'),
};

const withAccount: CloudStatusView = { configured: true, account: { plan: 'free' }, plan: 'free', servers: 'veydan' };
const withoutAccount: CloudStatusView = { configured: true, account: null, plan: null, servers: 'veydan' };

function usage(files: [number, number], text: [number, number] = [1, 512]): CloudUsageView {
  const pool = (name: string, [used, limit]: [number, number]) => ({
    pool: name,
    used,
    limit,
    percent: Math.min(100, Math.floor((used * 100) / limit)),
    level: 0,
  });
  return { pools: [pool('text', text), pool('files', files)], level: 0, profile_files_estimate: 800, on_cloud: true };
}

afterEach(() => vi.restoreAllMocks());

describe('Veydan Cloud in the sync settings', () => {
  it('is offered with an account, activated without one, absent without the module or on own servers', () => {
    expect(cloudChoice(true, withAccount)).toBe('option');
    expect(cloudChoice(true, withoutAccount)).toBe('activate');
    expect(cloudChoice(true, { ...withoutAccount, servers: 'unchosen' })).toBe('activate');
    expect(cloudChoice(false, withAccount)).toBe('none');
    expect(cloudChoice(true, null)).toBe('none');
    expect(cloudChoice(true, { ...withAccount, servers: 'own' })).toBe('none');
    expect(cloudChoice(true, { ...withAccount, servers: 'none' })).toBe('none');
    // The server did not answer; the device kept the account.
    const kept: CloudStatusView = { configured: false, account: null, plan: null, servers: 'veydan', kept: { plan: 'plus' } };
    expect(cloudChoice(true, kept)).toBe('option');
    expect(planOf(kept)).toBe('plus');
  });

  it('speaks of the room at 70, 90 and 100 percent', () => {
    expect(usageTone(0)).toBe('none');
    expect(usageTone(69)).toBe('none');
    expect(usageTone(70)).toBe('quiet');
    expect(usageTone(90)).toBe('warn');
    expect(usageTone(100)).toBe('full');
    expect(fullestPool(null)).toBeNull();
    expect(fullestPool(usage([1900, 2000]))?.pool).toBe('files');
    expect(fullestPool(usage([1, 2000], [500, 512]))?.pool).toBe('text');
    // The shared storage of the plan: the fullest, and the one a tie names.
    const shared = usage([2000, 2000]);
    shared.pools.push({ pool: 'total', used: 2001, limit: 2000, percent: 100, level: 100 });
    expect(fullestPool(shared)?.pool).toBe('total');
    expect(usageTone(fullestPool(shared)!.level)).toBe('full');
  });

  it('asks before the files of profiles go on the free plan only', () => {
    expect(asksForProfileFiles('cloud', 'free')).toBe(true);
    expect(asksForProfileFiles('cloud', 'plus')).toBe(false);
    expect(asksForProfileFiles('s3', 'free')).toBe(false);
    expect(filesLimit(usage([1, 2048]))).toBe(2048);
    expect(filesLimit(null)).toBeNull();
  });

  it('asks nothing where the module is off', async () => {
    setCloudSource(ofTheModule);
    vi.spyOn(registry, 'enabled').mockReturnValue(false);
    mockCommand('cloud_status', () => {
      throw new Error('not asked');
    });
    expect(await loadCloud()).toEqual({ choice: 'none', plan: null, usage: null });
  });

  it('asks nothing in a product without the module', async () => {
    setCloudSource(null);
    vi.spyOn(registry, 'enabled').mockReturnValue(true);
    expect(await loadCloud()).toEqual({ choice: 'none', plan: null, usage: null });
  });

  it('reads the status, then the usage of an account', async () => {
    setCloudSource(ofTheModule);
    vi.spyOn(registry, 'enabled').mockReturnValue(true);
    mockCommand('cloud_status', withAccount);
    mockCommand('cloud_usage', usage([10, 100]));
    const view = await loadCloud();
    expect(view.choice).toBe('option');
    expect(view.plan).toBe('free');
    expect(view.usage?.pools[1].percent).toBe(10);

    mockCommand('cloud_status', withoutAccount);
    mockCommand('cloud_usage', () => {
      throw new Error('not asked');
    });
    expect(await loadCloud()).toEqual({ choice: 'activate', plan: null, usage: null });

    // A usage that fails leaves the option, without numbers.
    mockCommand('cloud_status', withAccount);
    mockCommand('cloud_usage', () => {
      throw { code: 'transport', message: 'down', request_id: null };
    });
    expect(await loadCloud()).toEqual({ choice: 'option', plan: 'free', usage: null });
  });
});

/** The commands of sync as a device that syncs nowhere answers them; returns what was called, in order. */
function device(cfg: Partial<SyncConfig>, probe: 'empty' | 'vault' | 'foreign', joined = false) {
  const calls: string[] = [];
  let kept = {
    backend: 's3',
    folder_path: '',
    s3: { endpoint: '', bucket: '' },
    webdav: { url: '' },
    profile_files: true,
    ...cfg,
  } as SyncConfig;
  mockCommand('sync_status', () => ({ joined }));
  mockCommand('sync_get_config', () => kept);
  mockCommand('sync_set_config', (args: { cfg: SyncConfig }) => {
    calls.push(`set:${args.cfg.backend}:${args.cfg.profile_files}`);
    kept = args.cfg;
  });
  mockCommand('sync_probe', () => {
    calls.push('probe');
    return probe;
  });
  mockCommand('sync_join_vault', () => calls.push('join'));
  mockCommand('sync_create_vault', () => calls.push('create'));
  return calls;
}

describe('sync through Veydan Cloud by one button', () => {
  it('makes the account first, then joins the vault another device made', async () => {
    const calls = device({}, 'vault');
    setCloudSource({ ...ofTheModule, ensure: async () => void calls.push('ensure') });
    await connectCloudVault('plus');
    expect(calls).toEqual(['ensure', 'set:cloud:true', 'probe', 'join']);
  });

  it('creates the vault on the first device, the files of profiles off on the free plan', async () => {
    const calls = device({}, 'empty');
    setCloudSource(ofTheModule);
    await connectCloudVault('free');
    expect(calls).toEqual(['set:cloud:false', 'probe', 'create']);
  });

  it('turns nothing on when the account cannot be made', async () => {
    const calls = device({}, 'empty');
    setCloudSource({
      ...ofTheModule,
      ensure: async () => {
        throw new Error('no host');
      },
    });
    await expect(connectCloudVault('free')).rejects.toThrow('no host');
    expect(calls).toEqual([]);
  });
});

describe('the vault of the account at the first start', () => {
  it('is found on a device that syncs nowhere', async () => {
    const calls = device({}, 'vault');
    expect(await cloudVaultExists()).toBe(true);
    expect(calls).toEqual(['set:cloud:true', 'probe']);
    expect(await cloudVaultExists()).toBe(true);
    // The backend is `cloud` already: it is not written again.
    expect(calls).toEqual(['set:cloud:true', 'probe', 'probe']);
  });

  it('is not there on the first device, nor behind a host that does not answer', async () => {
    device({}, 'empty');
    expect(await cloudVaultExists()).toBe(false);
    device({}, 'vault');
    mockCommand('sync_probe', () => {
      throw new Error('down');
    });
    expect(await cloudVaultExists()).toBe(false);
  });

  it('is not asked of a device that is joined, or has a storage of its own filled in', async () => {
    let calls = device({}, 'vault', true);
    expect(await cloudVaultExists()).toBe(false);
    expect(calls).toEqual([]);
    calls = device({ backend: 'webdav', webdav: { url: 'https://dav.example.com' } as SyncConfig['webdav'] }, 'vault');
    expect(await cloudVaultExists()).toBe(false);
    expect(calls).toEqual([]);
  });
});

describe('the storage of an account made a moment ago', () => {
  const refusal = { code: 'other', message: 'auth: get: 403 Forbidden no_account: the signer has no account of Veydan Cloud (request 0bc5)' };

  it('is told from any other refusal by the code of the host', () => {
    expect(cloudNotReady(refusal)).toBe(true);
    expect(cloudNotReady(refusal.message)).toBe(true);
    expect(cloudNotReady({ code: 'other', message: 'auth: get: 403 Forbidden suspended' })).toBe(false);
    expect(cloudNotReady(new Error('network is down'))).toBe(false);
    expect(cloudNotReady(null)).toBe(false);
  });

  it('is waited for: the vault is asked again until the host knows the account', async () => {
    const calls = device({}, 'empty');
    setCloudSource(ofTheModule);
    let refusals = 2;
    mockCommand('sync_probe', () => {
      calls.push('probe');
      if (refusals-- > 0) throw refusal;
      return 'empty';
    });
    const onWait = vi.fn();
    const stages: string[] = [];
    await connectCloudVault('free', { onWait, onStage: (s) => stages.push(s), retryMs: 1 });
    expect(calls).toEqual(['set:cloud:false', 'probe', 'probe', 'probe', 'create']);
    // Said once, however many times the host was asked.
    expect(onWait).toHaveBeenCalledOnce();
    // The way as the screen shows it: the account, the storage for as long as it is not ready, then this device.
    expect(stages).toEqual(['account', 'storage', 'storage', 'storage', 'device']);
  });

  it('is waited for through the creation too, when another host answers it', async () => {
    const calls = device({}, 'empty');
    setCloudSource(ofTheModule);
    let first = true;
    mockCommand('sync_create_vault', () => {
      calls.push('create');
      if (first) {
        first = false;
        throw refusal;
      }
    });
    await connectCloudVault('free', { retryMs: 1 });
    expect(calls).toEqual(['set:cloud:false', 'probe', 'create', 'probe', 'create']);
  });

  it('is not waited for longer than the limit, nor at all after another refusal', async () => {
    device({}, 'empty');
    setCloudSource(ofTheModule);
    mockCommand('sync_probe', () => {
      throw refusal;
    });
    await expect(connectCloudVault('free', { limitMs: 10, retryMs: 2 })).rejects.toBe(refusal);

    const calls = device({}, 'empty');
    const down = { code: 'other', message: 'transport: connection refused' };
    mockCommand('sync_probe', () => {
      calls.push('probe');
      throw down;
    });
    await expect(connectCloudVault('free', { retryMs: 1 })).rejects.toBe(down);
    expect(calls).toEqual(['set:cloud:false', 'probe']);
  });
});
