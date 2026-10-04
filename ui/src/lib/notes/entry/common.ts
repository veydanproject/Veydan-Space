// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the notes module is regardless of the shell (platform-spec 11.1).

import type { ModuleDef } from '$lib/core/module';
import { noteEntities } from '$lib/notes/entities';
import { labelKinds } from '$lib/notes/mobile/labels.svelte';
import { notesStore } from '$lib/notes/store/notes.svelte';

export const common: Omit<ModuleDef, 'nav' | 'settings'> = {
  id: 'notes',
  title: 'nav_notes',
  icon: 'file-text',
  routes: ['/notes', '/search'],
  // Without the browser module the notes answer for workspaces and profiles from the labels they keep.
  entities: [...noteEntities, ...labelKinds],
  demo: true,
  // Attachments travel through the vault as large files.
  syncsFiles: true,
  // The shortcuts of the note editor (Settings → Hotkeys).
  hotkeys: ['editor'],
  reloaders: {
    note_tag: () => notesStore.loaded ? notesStore.refreshTags() : Promise.resolve(),
    note_folder: () => notesStore.loaded ? notesStore.refreshFolders() : Promise.resolve(),
    note_smart_view: () => notesStore.loaded ? notesStore.refreshSmartViews() : Promise.resolve(),
    note_meta: () => notesStore.loaded ? notesStore.refresh() : Promise.resolve(),
  },
};
