// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { profileDraft, type ProfileDraft } from './profileDraft';

const input = { name: 'a', display_name: 'A', about: 'x', website: null, nip05: null, lud16: null, socials: [] };
const draft = (owner: string): ProfileDraft => ({
  owner, start: input, form: { ...input, about: 'typed' }, about: 'typed', phone: '+1', share: true, privStart: null,
});

describe('profileDraft', () => {
  it('keeps a form for its identity until taken', () => {
    profileDraft.keep(draft('k1'));
    expect(profileDraft.has('k1')).toBe(true);
    expect(profileDraft.has('k2')).toBe(false);
    expect(profileDraft.take('k1')?.about).toBe('typed');
    expect(profileDraft.has('k1')).toBe(false);
  });

  it('drops a form of another identity and on clear', () => {
    profileDraft.keep(draft('k1'));
    expect(profileDraft.take('k2')).toBe(null);
    expect(profileDraft.has('k1')).toBe(false);
    profileDraft.keep(draft('k1'));
    profileDraft.clear();
    expect(profileDraft.take('k1')).toBe(null);
  });

  it('keeps a copy, not the live form', () => {
    const d = draft('k1');
    profileDraft.keep(d);
    d.form.about = 'changed later';
    expect(profileDraft.take('k1')?.form.about).toBe('typed');
  });
});
