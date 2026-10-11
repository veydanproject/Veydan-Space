// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The wizard of the first start (platform-spec 12): which steps a product
// has, when the flag of the first start goes, how a step of several
// screens takes Back, and how the wizard steps aside for a screen of the
// app and comes back.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Component } from 'svelte';

vi.mock('$app/navigation', () => ({ goto: vi.fn(async () => {}) }));

import { goto } from '$app/navigation';
import type { ModuleDef, OnboardingStep } from '../module';
import { registry } from '../registry';
import { modulesStore } from '../store/modules.svelte';
import { MODULES, WELCOME, isUnder, onboarding, orderSteps, pathOf } from './store.svelte';

const screen = {} as Component;
const step = (id: string, order: number, visible?: () => boolean): OnboardingStep => ({ id, title: 'ob_back', order, component: screen, visible });

function use(...defs: ModuleDef[]) {
  (registry as unknown as { defs: ModuleDef[] }).defs = defs;
}

const messenger: ModuleDef = {
  id: 'messenger',
  title: 'ob_back',
  icon: 'message-circle',
  routes: ['/messenger'],
  nav: [],
  settings: [],
  onboarding: [step('account', 20)],
};
const notes: ModuleDef = { id: 'notes', title: 'ob_back', icon: 'file-text', routes: ['/notes'], nav: [], settings: [] };

/** The core's steps as the wizard's component names them; the modules where the product has a choice. */
function core(choice: boolean) {
  onboarding.setCore([step(WELCOME, 0), step(MODULES, 10, () => choice), step('sync', 30)]);
}

beforeEach(() => {
  modulesStore.off = [];
  modulesStore.firstRun = true;
  onboarding.active = false;
});
afterEach(() => {
  use();
  vi.clearAllMocks();
});

describe('the steps of the first start', () => {
  it('are the core\'s and the modules\', those visible, in their order', () => {
    const steps = orderSteps([step('sync', 30), step('hidden', 5, () => false), step('account', 20), step(WELCOME, 0)]);
    expect(steps.map((s) => s.id)).toEqual([WELCOME, 'account', 'sync']);
  });

  it('have the account between the modules and sync, and lose it with the messenger', () => {
    use(notes, messenger);
    core(true);
    expect(onboarding.steps.map((s) => s.id)).toEqual([WELCOME, MODULES, 'account', 'sync']);
    expect(onboarding.has('account')).toBe(true);
    modulesStore.off = ['messenger'];
    expect(onboarding.steps.map((s) => s.id)).toEqual([WELCOME, MODULES, 'sync']);
    expect(onboarding.has('account')).toBe(false);
  });

  it('count from the step after the welcome', () => {
    use(notes, messenger);
    core(true);
    onboarding.open();
    expect(onboarding.position).toBe(0);
    expect(onboarding.counted).toHaveLength(3);
  });
});

describe('the flag of the first start', () => {
  it('opens the wizard, and only it', () => {
    use(notes, messenger);
    core(true);
    modulesStore.firstRun = false;
    onboarding.begin();
    expect(onboarding.active).toBe(false);
    modulesStore.firstRun = true;
    onboarding.begin();
    expect(onboarding.active).toBe(true);
    expect(onboarding.current).toBe(WELCOME);
  });

  it('stays for the step of the modules, whose answer takes it away', async () => {
    use(notes, messenger);
    core(true);
    onboarding.open();
    await onboarding.next();
    expect(onboarding.current).toBe(MODULES);
    expect(modulesStore.firstRun).toBe(true);
  });

  it('goes with the welcome where there is no choice of modules', async () => {
    use(messenger);
    core(false);
    onboarding.open();
    await onboarding.next();
    expect(onboarding.current).toBe('account');
    expect(modulesStore.firstRun).toBe(false);
  });

  it('goes when the welcome leads straight to the account the person has', async () => {
    use(notes, messenger);
    core(true);
    onboarding.open();
    await onboarding.go('account', 'have');
    expect(onboarding.current).toBe('account');
    expect(onboarding.intent).toBe('have');
    expect(modulesStore.firstRun).toBe(false);
    // The next step starts without what the last one was asked.
    await onboarding.next();
    expect(onboarding.intent).toBeNull();
  });

  it('goes at the end, and the app opens at its start', async () => {
    use(notes);
    core(false);
    onboarding.open();
    await onboarding.next();
    expect(onboarding.current).toBe('sync');
    await onboarding.next();
    expect(onboarding.active).toBe(false);
    expect(goto).toHaveBeenCalledWith('/', { replaceState: true });
  });
});

describe('Back', () => {
  it('goes to the step before', async () => {
    use(notes, messenger);
    core(true);
    onboarding.open();
    await onboarding.go('sync');
    onboarding.back();
    expect(onboarding.current).toBe('account');
  });

  it('is the step\'s own while it is on an inner screen, and no longer after the step', async () => {
    use(notes, messenger);
    core(true);
    onboarding.open();
    await onboarding.go('account');
    let inner = true;
    onboarding.takeBack(() => {
      const was = inner;
      inner = false;
      return was;
    });
    onboarding.back();
    expect(onboarding.current).toBe('account');
    onboarding.back();
    expect(onboarding.current).toBe(MODULES);
    // A handler of the step left behind is not asked again.
    inner = true;
    await onboarding.go('sync');
    onboarding.back();
    expect(onboarding.current).toBe('account');
  });
});

describe('a screen of the app in the middle of the wizard', () => {
  it('is named by its path', () => {
    expect(pathOf('/cloud?section=recovery-key')).toBe('/cloud');
    expect(pathOf('/settings#sync')).toBe('/settings');
    expect(isUnder('/cloud/link', '/cloud')).toBe(true);
    expect(isUnder('/cloudy', '/cloud')).toBe(false);
  });

  it('takes the wizard out of the way until the person leaves it', () => {
    use(notes);
    core(false);
    onboarding.open();
    onboarding.detour('/settings/sync');
    expect(goto).toHaveBeenCalledWith('/settings/sync');
    // The navigation has not arrived yet: the wizard stays, so the page under it is not shown on the way.
    expect(onboarding.hidden).toBe(false);
    onboarding.at('/');
    expect(onboarding.hidden).toBe(false);
    onboarding.at('/settings/sync');
    expect(onboarding.hidden).toBe(true);
    onboarding.at('/settings');
    expect(onboarding.hidden).toBe(false);
    expect(onboarding.active).toBe(true);
  });
});

describe('the wizard opened again from the card «Finish setting up»', () => {
  it('opens at the step asked for and goes on to the end, leaving the page it was opened over', async () => {
    use(notes, messenger);
    core(true);
    modulesStore.firstRun = false;
    expect(onboarding.resume('account')).toBe(true);
    expect(onboarding.active).toBe(true);
    expect(onboarding.current).toBe('account');
    // Nothing before the step it was opened at.
    onboarding.back();
    expect(onboarding.current).toBe('account');
    await onboarding.next();
    expect(onboarding.current).toBe('sync');
    // Past the step it was opened at, Back works as ever.
    onboarding.back();
    expect(onboarding.current).toBe('account');
    await onboarding.next();
    await onboarding.next();
    expect(onboarding.active).toBe(false);
    expect(onboarding.floor).toBeNull();
    expect(goto).not.toHaveBeenCalled();
  });

  it('does not open at a step the product has not now, and closes where it stands', () => {
    use(notes);
    core(false);
    expect(onboarding.resume('account')).toBe(false);
    expect(onboarding.active).toBe(false);
    expect(onboarding.resume('sync')).toBe(true);
    onboarding.close();
    expect(onboarding.active).toBe(false);
    expect(goto).not.toHaveBeenCalled();
    // The first start after that is a whole one again.
    onboarding.open();
    expect(onboarding.floor).toBeNull();
    expect(onboarding.current).toBe(WELCOME);
  });
});
