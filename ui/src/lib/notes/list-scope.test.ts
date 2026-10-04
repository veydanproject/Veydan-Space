// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { noteScope, scopeColor, type ScopeSources } from './list-scope';

const all: ScopeSources = {
  workspaceName: (id) => `ws ${id}`,
  workspaceColor: (id) => (id === 'w1' ? '#f472b6' : 'var(--success)'),
  profileName: (id) => `p ${id}`,
  folderName: (id) => `f ${id}`,
  folderColor: (id) => (id === 'f1' ? '#60a5fa' : 'var(--text-2)'),
};

describe('list-scope', () => {
  it('names a folder first, then the workspace, then the profile', () => {
    expect(noteScope({ folder_ids: ['f1', 'f2'], bindings: ['workspace:w1'] }, all)).toEqual({ kind: 'folder', id: 'f1', more: 1 });
    expect(noteScope({ folder_ids: [], bindings: ['profile:p1', 'workspace:w1'] }, all)).toEqual({ kind: 'workspace', id: 'w1', binding: 'workspace:w1' });
    expect(noteScope({ folder_ids: [], bindings: ['profile:p1'] }, all)).toEqual({ kind: 'profile', id: 'p1', binding: 'profile:p1' });
    expect(noteScope({ folder_ids: [], bindings: ['domain:example.com'] }, all)).toEqual({ kind: 'global' });
  });

  it('names only what it has a name for', () => {
    expect(noteScope({ folder_ids: ['f1'], bindings: ['workspace:w1'] }, { workspaceName: all.workspaceName })).toMatchObject({ kind: 'workspace' });
    expect(noteScope({ folder_ids: [], bindings: ['workspace:w1'] }, {})).toEqual({ kind: 'global' });
  });

  it('gives the workspace its own colour, the one of the sidebar and the card', () => {
    const scope = noteScope({ folder_ids: [], bindings: ['workspace:w1'] }, all);
    expect(scopeColor(scope, all)).toBe('#f472b6');
  });

  it('falls back to the green of a workspace without a colour', () => {
    const scope = noteScope({ folder_ids: [], bindings: ['workspace:w2'] }, all);
    expect(scopeColor(scope, all)).toBe('var(--success)');
    expect(scopeColor(scope, { workspaceName: all.workspaceName })).toBe('var(--success)');
  });

  it('keeps the folder, profile and global colours', () => {
    expect(scopeColor(noteScope({ folder_ids: ['f1'], bindings: [] }, all), all)).toBe('#60a5fa');
    expect(scopeColor(noteScope({ folder_ids: [], bindings: ['profile:p1'] }, all), all)).toBe('var(--accent)');
    expect(scopeColor(noteScope({ folder_ids: [], bindings: [] }, all), all)).toBe('var(--text-2)');
  });
});
