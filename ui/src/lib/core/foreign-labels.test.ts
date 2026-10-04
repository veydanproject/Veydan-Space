// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The names of references without an owner in the product (platform-spec
// 10.3): the catalog asks `labels_resolve` once per reference, in one call
// per tick, and shows the kind and a short id only where there is no label.

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { mockCommand, type LabelName, type LabelRef } from './api';
import { directory, foreignName } from './directory';
import { foreignLabels, hexColor, LABELS_BATCH } from './foreign-labels.svelte';
import type { EntityKindDef, ModuleDef } from './module';
import { registry } from './registry';
import { modulesStore } from './store/modules.svelte';

/** What the backend's `labels` table holds. */
let labels: Record<string, string>;
/** The colors of those labels, as the table keeps them (sync may bring anything). */
let colors: Record<string, string>;
/** The batches `labels_resolve` was called with. */
let calls: LabelRef[][];
let fail = false;

function use(...defs: ModuleDef[]) {
  (registry as unknown as { defs: ModuleDef[] }).defs = defs;
  modulesStore.apply({ modules: defs.map((d) => ({ id: d.id, enabled: true })), first_run: false });
}

const kind = (k: string, entities: { id: string; name: string; color?: string }[], fallback?: boolean): EntityKindDef => ({
  kind: k,
  icon: 'lock',
  label: 'ctx_kind_password',
  color: '',
  fallback,
  ensureLoaded: async () => {},
  list: () => entities.map((e) => ({ ...e, subtitle: '', status: null, color: e.color ?? '' })),
  actions: [],
});

/** A product like Pass: it owns the passwords and nothing a password links to. */
const pass: ModuleDef = {
  id: 'pass',
  title: 'ctx_kind_password',
  icon: 'lock',
  routes: ['/'],
  nav: [],
  settings: [],
  entities: [kind('password', [{ id: 'pw-1', name: 'Mail' }])],
};

const tr = (key: string) => ({ ctx_kind_profile: 'Profile', ctx_kind_workspace: 'Workspace' })[key] ?? key;

/** The microtask of the batch and the answer after it. */
const settled = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
  labels = { 'profile:0a1b2c3d-pr-1': 'Brand A', 'workspace:ws-1': 'SMM' };
  colors = { 'workspace:ws-1': '#22c55e' };
  calls = [];
  fail = false;
  mockCommand('labels_resolve', (args: { items: LabelRef[] }): LabelName[] => {
    calls.push(args.items);
    if (fail) throw new Error('no backend');
    return args.items.map(({ kind, id }) => ({ kind, id, name: labels[`${kind}:${id}`] ?? null, color: colors[`${kind}:${id}`] ?? null }));
  });
  use(pass);
});

afterEach(() => {
  foreignLabels.forget();
  use();
});

describe('a reference to a kind without an owner in the product', () => {
  it('is shown by its kind and short id until the label answers, then by its name', async () => {
    const before = directory.foreign('profile', '0a1b2c3d-pr-1');
    expect(before).toMatchObject({ kind: 'profile', icon: 'globe', kindLabel: 'ctx_kind_profile', name: null });
    expect(foreignName(before!, tr)).toBe('Profile 0a1b2c3d');
    expect(directory.linkName('profile', '0a1b2c3d-pr-1')).toBeUndefined();

    await settled();
    const after = directory.foreign('profile', '0a1b2c3d-pr-1')!;
    expect(after.name).toBe('Brand A');
    expect(foreignName(after, tr)).toBe('Brand A');
    expect(directory.name('profile', '0a1b2c3d-pr-1', tr)).toBe('Brand A');
    expect(directory.linkName('profile', '0a1b2c3d-pr-1')).toBe('Brand A');
  });

  it('keeps the kind and the short id when there is no label', async () => {
    directory.foreign('profile', 'ffffffff-gone');
    await settled();
    const gone = directory.foreign('profile', 'ffffffff-gone')!;
    expect(gone.name).toBeNull();
    expect(directory.name('profile', 'ffffffff-gone', tr)).toBe('Profile ffffffff');
    expect(directory.linkName('profile', 'ffffffff-gone')).toBeUndefined();
  });

  it('asks for the references of one tick in one call, and for each once', async () => {
    directory.foreign('profile', '0a1b2c3d-pr-1');
    directory.foreign('workspace', 'ws-1');
    directory.foreign('profile', '0a1b2c3d-pr-1');
    directory.name('note', 'n-1', tr);
    await settled();
    expect(calls).toEqual([
      [
        { kind: 'profile', id: '0a1b2c3d-pr-1' },
        { kind: 'workspace', id: 'ws-1' },
        { kind: 'note', id: 'n-1' },
      ],
    ]);
    directory.foreign('workspace', 'ws-1');
    directory.foreign('note', 'n-1');
    await settled();
    expect(calls).toHaveLength(1);
  });

  it('asks in batches the command accepts, however many references there are', async () => {
    const count = LABELS_BATCH * 2 + 5;
    for (let i = 0; i < count; i++) directory.foreign('profile', `pr-${i}`);
    labels[`profile:pr-${count - 1}`] = 'Last';
    await settled();
    expect(calls.map((c) => c.length)).toEqual([LABELS_BATCH, LABELS_BATCH, 5]);
    expect(directory.foreign('profile', `pr-${count - 1}`)!.name).toBe('Last');
    calls = [];
    foreignLabels.refresh();
    await settled();
    expect(calls.map((c) => c.length)).toEqual([LABELS_BATCH, LABELS_BATCH, 5]);
  });

  it('takes a name that came later, and a new one, on a refresh', async () => {
    directory.foreign('proxy', 'px-1');
    directory.foreign('workspace', 'ws-1');
    await settled();
    expect(directory.foreign('proxy', 'px-1')!.name).toBeNull();

    labels['proxy:px-1'] = 'DE residential';
    labels['workspace:ws-1'] = 'Agency';
    foreignLabels.refresh();
    await settled();
    expect(directory.foreign('proxy', 'px-1')!.name).toBe('DE residential');
    expect(directory.foreign('workspace', 'ws-1')!.name).toBe('Agency');

    // The owner retracted the label: the reference goes back to its kind and id.
    delete labels['proxy:px-1'];
    foreignLabels.refresh();
    await settled();
    expect(directory.name('proxy', 'px-1', tr)).toBe('ctx_kind_proxy px-1');
  });

  it('stays without a name when the backend does not answer, and asks again on a refresh', async () => {
    fail = true;
    directory.foreign('workspace', 'ws-1');
    await settled();
    expect(directory.name('workspace', 'ws-1', tr)).toBe('Workspace ws-1');
    fail = false;
    foreignLabels.refresh();
    await settled();
    expect(directory.name('workspace', 'ws-1', tr)).toBe('SMM');
  });

  it('takes an answer only for what it asked, and an empty name as none', async () => {
    labels['workspace:ws-1'] = '  ';
    mockCommand('labels_resolve', (args: { items: LabelRef[] }): LabelName[] => [
      ...args.items.map(({ kind, id }) => ({ kind, id, name: labels[`${kind}:${id}`] ?? null, color: null })),
      { kind: 'note', id: 'n-9', name: 'Not asked', color: '#123456' },
    ]);
    directory.foreign('workspace', 'ws-1');
    await settled();
    expect(directory.foreign('workspace', 'ws-1')!.name).toBeNull();
    expect(foreignLabels.name('workspace', 'ws-1')).toBeNull();
    // `n-9` is asked for now; the earlier answer did not name it.
    expect(directory.foreign('note', 'n-9')!.name).toBeNull();
  });
});

describe('a reference the labels are not asked for', () => {
  it('is one whose kind a module of the product owns', async () => {
    expect(directory.foreign('password', 'pw-1')).toBeUndefined();
    expect(directory.name('password', 'pw-1', tr)).toBe('Mail');
    expect(directory.linkName('password', 'pw-1')).toBe('Mail');
    // An owned entity that is gone is shown by its id, as before.
    expect(directory.name('password', 'pw-2', tr)).toBe('pw-2');
    await settled();
    expect(calls).toEqual([]);
  });

  it('is one that is not a kind of entity, or has no id', async () => {
    expect(directory.foreign('domain', 'example.com')).toBeUndefined();
    expect(directory.foreign('profile', '')).toBeUndefined();
    await settled();
    expect(calls).toEqual([]);
  });

  it('is one a fallback of the product already names', async () => {
    const notes: ModuleDef = { ...pass, id: 'notes', entities: [kind('workspace', [{ id: 'ws-1', name: 'From nav' }], true)] };
    use(notes);
    expect(directory.foreign('workspace', 'ws-1')!.name).toBe('From nav');
    await settled();
    expect(calls).toEqual([]);
    // What the fallback does not list is asked for like any other.
    expect(directory.foreign('workspace', 'ws-2')!.name).toBeNull();
    await settled();
    expect(calls).toEqual([[{ kind: 'workspace', id: 'ws-2' }]]);
  });
});

describe('the color of a reference without an owner in the product', () => {
  it('is the color its label keeps, asked with the name in the same call', async () => {
    expect(directory.foreign('workspace', 'ws-1')!.color).toBeNull();
    expect(directory.color('workspace', 'ws-1')).toBeUndefined();
    await settled();
    expect(directory.foreign('workspace', 'ws-1')).toMatchObject({ name: 'SMM', color: '#22c55e' });
    expect(directory.color('workspace', 'ws-1')).toBe('#22c55e');
    expect(calls).toEqual([[{ kind: 'workspace', id: 'ws-1' }]]);
    // A profile's label has no color: none.
    directory.foreign('profile', '0a1b2c3d-pr-1');
    await settled();
    expect(directory.color('profile', '0a1b2c3d-pr-1')).toBeUndefined();
  });

  it('follows a recolor on a refresh, and goes with the label', async () => {
    directory.color('workspace', 'ws-1');
    await settled();
    colors['workspace:ws-1'] = '#f97316';
    foreignLabels.refresh();
    await settled();
    expect(directory.color('workspace', 'ws-1')).toBe('#f97316');
    delete labels['workspace:ws-1'];
    foreignLabels.refresh();
    await settled();
    expect(directory.foreign('workspace', 'ws-1')).toMatchObject({ name: null, color: null });
    expect(directory.color('workspace', 'ws-1')).toBeUndefined();
  });

  it('is never anything but a hex color, and none for a link without a name', async () => {
    const bad = ['red', '#22c55e; background: url(x)', 'var(--accent)', '22c55e', '#22c55e0', '#ggg', ' #22c55e', ''];
    bad.forEach((color, i) => {
      labels[`workspace:bad-${i}`] = 'WS';
      colors[`workspace:bad-${i}`] = color;
      directory.foreign('workspace', `bad-${i}`);
    });
    labels['workspace:short'] = 'WS';
    colors['workspace:short'] = '#ABC';
    colors['workspace:nameless'] = '#123456';
    directory.foreign('workspace', 'short');
    directory.foreign('workspace', 'nameless');
    await settled();
    bad.forEach((color, i) => expect(directory.color('workspace', `bad-${i}`), color).toBeUndefined());
    expect(directory.color('workspace', 'short')).toBe('#ABC');
    expect(directory.color('workspace', 'nameless')).toBeUndefined();
  });

  it('is the owner\'s where a module owns the kind, and is not asked', async () => {
    const space: ModuleDef = { ...pass, id: 'browser', entities: [kind('workspace', [{ id: 'ws-1', name: 'SMM', color: '#6366f1' }])] };
    use(space);
    expect(directory.color('workspace', 'ws-1')).toBe('#6366f1');
    expect(directory.color('workspace', 'ws-gone')).toBeUndefined();
    await settled();
    expect(calls).toEqual([]);
  });

  it('is the one a fallback lists, a hex color only, without asking', async () => {
    const notes: ModuleDef = {
      ...pass,
      id: 'notes',
      entities: [kind('workspace', [{ id: 'ws-1', name: 'From nav', color: '#0ea5e9' }, { id: 'ws-2', name: 'Odd', color: 'var(--success)' }], true)],
    };
    use(notes);
    expect(directory.color('workspace', 'ws-1')).toBe('#0ea5e9');
    expect(directory.color('workspace', 'ws-2')).toBeUndefined();
    await settled();
    expect(calls).toEqual([]);
  });
});

describe('hexColor', () => {
  it('takes #rgb and #rrggbb as they are, and nothing else', () => {
    for (const ok of ['#fff', '#FFF', '#6366f1', '#8B7BFF']) expect(hexColor(ok)).toBe(ok);
    for (const bad of [null, undefined, 0, '', '#', '#ff', '#ffff', '#fffffff', 'fff', '#ggg', '#fff ', 'red', 'url(x)', '#ff0000)']) {
      expect(hexColor(bad), String(bad)).toBeNull();
    }
  });
});
