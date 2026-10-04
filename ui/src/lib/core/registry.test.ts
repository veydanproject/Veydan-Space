// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The registry and the catalog over hand-made module descriptions: what a
// product of one module, a service and a fallback provider change
// (platform-spec 10.3, 11.7, section 12).

import { afterEach, describe, expect, it } from 'vitest';
import type { Component } from 'svelte';
import type { EntityKindDef, ModuleDef } from './module';
import { leavesOff, registry } from './registry';
import { directory } from './directory';
import { commands } from './commands';
import { modulesStore } from './store/modules.svelte';
import { rootNav } from './mobile/nav';

const card = {} as Component;

function use(...defs: ModuleDef[]) {
  (registry as unknown as { defs: ModuleDef[] }).defs = defs;
}

const kind = (k: string, entities: { id: string; name: string }[], fallback?: boolean): EntityKindDef => ({
  kind: k,
  icon: 'layers',
  label: 'ctx_kind_workspace',
  color: '',
  fallback,
  ensureLoaded: async () => {},
  list: () => entities.map((e) => ({ ...e, subtitle: '', status: null, color: '' })),
  actions: [],
});

const notes: ModuleDef = {
  id: 'notes',
  title: 'ctx_kind_note',
  icon: 'file-text',
  routes: ['/notes', '/search'],
  nav: [{ id: 'notes', title: 'ctx_kind_note', icon: 'file-text', href: '/notes' }],
  settings: [],
  entities: [kind('workspace', [{ id: 'ws-1', name: 'Work WS' }], true)],
};
const browser: ModuleDef = {
  id: 'browser',
  title: 'ctx_kind_workspace',
  icon: 'layers',
  routes: ['/', '/workspace'],
  nav: [],
  settings: [],
  home: card,
  entities: [kind('workspace', [{ id: 'ws-2', name: 'Owned WS' }])],
};
const chat: ModuleDef = {
  id: 'messenger',
  title: 'ctx_kind_note',
  icon: 'message-circle',
  routes: ['/messenger'],
  nav: [],
  settings: [{ id: 'messenger', title: 'ctx_kind_note', group: 'ctx_kind_note', order: 60, component: card }],
};
const backup: ModuleDef = {
  id: 'backup',
  title: 'ctx_kind_note',
  icon: 'archive',
  routes: [],
  nav: [],
  settings: [{ id: 'backup', title: 'ctx_kind_note', group: 'ctx_kind_note', order: 50, component: card }],
  service: true,
};

const tr = (key: string) => (key === 'ctx_kind_workspace' ? 'Workspace' : key);

/** The switches as the shell sends them: `off` are switched off. */
function switches(ids: string[], off: string[] = [], firstRun = false) {
  modulesStore.apply({ modules: ids.map((id) => ({ id, enabled: !off.includes(id) })), first_run: firstRun });
}

afterEach(() => {
  use();
  switches([]);
});

describe('the registry', () => {
  it('knows a product of one module, services aside', () => {
    use(notes);
    expect(registry.only?.id).toBe('notes');
    use(backup, notes);
    expect(registry.only?.id).toBe('notes');
    use(browser, notes, chat, backup);
    expect(registry.only).toBeUndefined();
  });

  it('never opens a service at `/`, and a module at its first screen of its own', () => {
    use(backup, notes);
    expect(registry.defaultModule()?.id).toBe('notes');
    use(browser, notes);
    expect(registry.defaultModule()?.id).toBe('browser');
    expect(registry.homeRoute(browser)).toBe('/workspace');
    expect(registry.homeRoute(notes)).toBe('/notes');
  });

  it('lists the switches of the modules, never of a service, and hides the cards of a module that is off', () => {
    use(notes, chat, backup);
    expect(registry.switchable().map((m) => m.id)).toEqual(['notes', 'messenger']);
    expect(registry.settings().map((s) => s.id)).toEqual(['messenger', 'backup']);
    switches(['notes', 'messenger'], ['messenger']);
    expect(registry.settings().map((s) => s.id)).toEqual(['backup']);
    // The shell lists the modules with a switch; one this platform lacks is not shown.
    switches(['browser', 'notes', 'messenger'], ['browser']);
    expect(registry.switchable().map((m) => m.id)).toEqual(['notes', 'messenger']);
  });

  it('follows the switches in the navigation, at `/`, in the routes and in the palette', () => {
    const pass: ModuleDef = {
      id: 'pass',
      title: 'ctx_kind_note',
      icon: 'lock',
      routes: ['/passwords'],
      nav: [{ id: 'pass', title: 'ctx_kind_note', icon: 'lock', href: '/passwords' }],
      settings: [],
      commands: () => commands.register({ id: 'test.pass', title: 'Pass', group: 'test', run: () => {} }),
    };
    use(browser, notes, pass, backup);
    commands.as('pass', () => pass.commands?.());
    switches(['browser', 'notes', 'pass']);
    expect(registry.enabled('pass')).toBe(true);
    expect(registry.nav().map((n) => n.id)).toEqual(['notes', 'pass']);
    expect(registry.defaultModule()?.id).toBe('browser');
    expect(commands.all().map((c) => c.id)).toContain('test.pass');
    switches(['browser', 'notes', 'pass'], ['browser', 'pass']);
    expect(registry.enabled('pass')).toBe(false);
    expect(registry.enabled('backup')).toBe(true);
    expect(registry.nav().map((n) => n.id)).toEqual(['notes']);
    expect(registry.defaultModule()?.id).toBe('notes');
    expect(commands.all().map((c) => c.id)).not.toContain('test.pass');
    expect(leavesOff('/passwords/x')).toBe(true);
    expect(leavesOff('/notes')).toBe(false);
    // `/` opens the default module, which is on.
    expect(leavesOff('/')).toBe(false);
  });

  it('starts a module switched on and stops one switched off, once a shell started them', async () => {
    const calls: string[] = [];
    const tracked = (m: ModuleDef): ModuleDef => ({
      ...m,
      start: async () => void calls.push(`start ${m.id}`),
      stop: () => void calls.push(`stop ${m.id}`),
    });
    use(tracked(notes), tracked(chat));
    switches(['notes', 'messenger'], ['messenger']);
    await registry.startAll();
    expect(calls).toEqual(['start notes']);
    switches(['notes', 'messenger']);
    await registry.follow();
    switches(['notes', 'messenger'], ['notes']);
    await registry.follow();
    expect(calls).toEqual(['start notes', 'start messenger', 'stop notes']);
    registry.stopAll();
    expect(calls.at(-1)).toBe('stop messenger');
    // Stopped by the shell, a switch starts nothing.
    switches(['notes', 'messenger']);
    await Promise.resolve();
    expect(calls.at(-1)).toBe('stop messenger');
  });

  it('takes the search screen from the module that has one', () => {
    use(chat);
    expect(registry.search()).toBeUndefined();
    use(chat, { ...notes, search: '/search' });
    expect(registry.search()).toBe('/search');
  });

  it('gives a product of one module its own screens on the phone instead of Home (11.7)', () => {
    use(backup, notes);
    expect(rootNav().map((n) => n.id)).toEqual(['notes', 'settings']);
    expect(rootNav()[0].href).toBe('/notes');
    use(browser, notes, chat, backup);
    expect(rootNav().map((n) => n.id)).toEqual(['home', 'settings']);
    expect(rootNav()[0].href).toBe('/');
  });

  it('leaves out quick access to a module where the module is the whole product (Pass)', () => {
    const pass: ModuleDef = {
      ...chat,
      id: 'pass',
      tools: [
        { id: 'drawer', title: 'ctx_kind_note', icon: 'lock', quick: true, onclick: () => {} },
        { id: 'window', title: 'ctx_kind_note', icon: 'lock', onclick: () => {} },
      ],
      overlays: [
        { id: 'drawers', component: card, quick: true },
        { id: 'banner', component: card },
      ],
    };
    use(backup, pass);
    expect(registry.tools().map((tool) => tool.id)).toEqual(['window']);
    expect(registry.overlays('window').map((o) => o.key)).toEqual(['pass:banner']);
    use(browser, pass);
    expect(registry.tools().map((tool) => tool.id)).toEqual(['drawer', 'window']);
    expect(registry.overlays('window').map((o) => o.key)).toEqual(['pass:drawers', 'pass:banner']);
  });
});

describe('the catalog with a fallback provider', () => {
  it('answers a kind no module owns from the fallback, as 10.3 shows it', () => {
    use(notes);
    expect(directory.get('workspace')?.fallback).toBe(true);
    expect(directory.owned('workspace')).toBe(false);
    expect(directory.list('workspace').map((w) => w.name)).toEqual(['Work WS']);
    expect(directory.name('workspace', 'ws-1', tr)).toBe('Work WS');
    expect(directory.foreign('workspace', 'ws-1')?.name).toBe('Work WS');
    // No name yet: the kind's name and a short id.
    expect(directory.name('workspace', '0123456789abcdef', tr)).toBe('Workspace 01234567');
  });

  it('lets the owner answer where it is present', () => {
    use(browser, notes);
    expect(directory.kinds.filter((d) => d.kind === 'workspace')).toHaveLength(1);
    expect(directory.owned('workspace')).toBe(true);
    expect(directory.list('workspace').map((w) => w.name)).toEqual(['Owned WS']);
    expect(directory.foreign('workspace', 'ws-1')).toBeUndefined();
    // A reference to an owned entity that is gone keeps showing its id, as before.
    expect(directory.name('workspace', 'ws-1', tr)).toBe('ws-1');
  });
});
