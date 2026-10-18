// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The scope line of a note's row in the list: what it names (a folder, else
// the workspace, else the profile, else nothing) and the colour it takes. The
// Notes page and the workspace's Notes drawer share it; a folder and a
// workspace have the colour the user gave them (the one of the sidebar and
// the chip); a profile, or one with no colour, none: a colour is the user's,
// never a kind's.

import type { NoteListItem } from '$lib/notes/types';

export interface ScopeSources {
  workspaceName?: (id: string) => string;
  workspaceColor?: (id: string) => string | undefined;
  profileName?: (id: string) => string;
  folderName?: (id: string) => string;
  folderColor?: (id: string) => string | undefined;
}

export type NoteScope =
  | { kind: 'folder'; id: string; more: number }
  | { kind: 'workspace' | 'profile'; id: string; binding: string }
  | { kind: 'global' };

type ScopeNote = Pick<NoteListItem, 'folder_ids' | 'bindings'>;

/** What the scope line names: only what a name can be given for. */
export function noteScope(n: ScopeNote, s: ScopeSources): NoteScope {
  if (n.folder_ids.length > 0 && s.folderName) return { kind: 'folder', id: n.folder_ids[0], more: n.folder_ids.length - 1 };
  const ws = n.bindings.find((b) => b.startsWith('workspace:'));
  if (ws && s.workspaceName) return { kind: 'workspace', id: ws.slice('workspace:'.length), binding: ws };
  const profile = n.bindings.find((b) => b.startsWith('profile:'));
  if (profile && s.profileName) return { kind: 'profile', id: profile.slice('profile:'.length), binding: profile };
  return { kind: 'global' };
}

/** The colour of the scope line: the folder's or the workspace's own; none (neutral) otherwise. */
export function scopeColor(scope: NoteScope, s: ScopeSources): string | undefined {
  switch (scope.kind) {
    case 'folder': return s.folderColor?.(scope.id) || undefined;
    case 'workspace': return s.workspaceColor?.(scope.id) || undefined;
    default: return undefined;
  }
}
