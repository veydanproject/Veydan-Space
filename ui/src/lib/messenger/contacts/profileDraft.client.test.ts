// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The draft with the state the app really has: the tests compile for the
// server, where $state is a plain object, while the client's is a proxy that
// structuredClone refuses. This file is plain .ts (Svelte forbids its internal
// runtime in .svelte.ts), and makes the state with a module compiled for the
// client; no DOM is needed.

import { compileModule } from 'svelte/compiler';
// @ts-expect-error the client runtime is private to Svelte and has no types
import * as client from 'svelte/internal/client';
import { describe, expect, it } from 'vitest';
import { profileDraft, type ProfileDraft } from './profileDraft.svelte';

/** `$state(v)` as the client makes it. */
function clientState(): <T extends object>(v: T) => T {
  const { code } = compileModule('export const state = (v) => { let s = $state(v); return s; };', {
    filename: 'state.svelte.js',
    generate: 'client',
  }).js;
  const body = code.replace(/^import .*$/m, '').replace('export const state =', 'const state =');
  return new Function('$', `${body}\nreturn state;`)(client);
}

const input = { name: 'a', display_name: 'A', about: 'x', website: null, nip05: null, lud16: null, socials: [], bot: false, commands: [] };
const draft = (start: ProfileDraft['start'], form: ProfileDraft['form']): ProfileDraft => ({
  owner: 'k1', start, form, about: 'typed', phone: '+1', share: true, privStart: null,
});

describe('profileDraft with client-side $state', () => {
  it('keeps a form made of real $state and gives it back', () => {
    const state = clientState();
    const start = state({ ...input, socials: [{ p: 'x', h: 'a' }], commands: [{ name: 'c', about: 'd' }] });
    const form = state({ ...input, about: 'old', socials: [{ p: 'x', h: 'b' }] });
    expect(() => structuredClone(form)).toThrow();
    form.about = 'typed';
    form.socials[0].h = 'edited';
    form.socials.push({ p: 'y', h: 'c' });

    expect(() => profileDraft.keep(draft(start, form))).not.toThrow();

    // Later edits of the live state do not reach what was kept.
    form.socials[0].h = 'later';
    start.commands[0].name = 'later';
    const back = profileDraft.take('k1');
    expect(back?.form.about).toBe('typed');
    expect(back?.form.socials).toEqual([{ p: 'x', h: 'edited' }, { p: 'y', h: 'c' }]);
    expect(back?.start.commands).toEqual([{ name: 'c', about: 'd' }]);
    expect(() => structuredClone(back)).not.toThrow();
  });
});
