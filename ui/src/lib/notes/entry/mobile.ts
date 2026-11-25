// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The notes module as the mobile shell sees it (platform-spec 11.1).
// A wide window shows a note beside the list it was opened from.

import type { ModuleDef, SplitDef } from '$lib/core/module';
import { common } from './common';
import { notesBar } from '$lib/notes/mobile/nav';
import NotesPage from '$lib/notes/mobile/NotesPage.svelte';
import NotesMoreOverlay from '$lib/notes/mobile/NotesMoreOverlay.svelte';
import EntityLabelField from '$lib/notes/mobile/EntityLabelField.svelte';
import AttachmentPolicyBlock from '$lib/notes/mobile/AttachmentPolicyBlock.svelte';

/**
 * The list a note stands beside (`/notes/:id`, `/notes/new`), the list
 * itself at `/notes` whatever its filter; one object, so the shell keeps it
 * mounted while the filter changes and a note opens. Beside a note the list
 * shows the filter it showed last (`notesMobileUi.listHref`). The tags: a
 * screen as on a phone.
 */
const LIST: SplitDef = { home: '/notes', list: NotesPage, empty: { icon: 'notes', title: 'note_empty_hint' } };
export function splitOf(url: URL): SplitDef | undefined {
  const path = url.pathname.replace(/\/$/, '');
  return path === '/notes' || /^\/notes\/(?!tags$)[^/]+$/.test(path) ? LIST : undefined;
}

export const module: ModuleDef = {
  ...common,
  nav: [{ id: 'notes', title: 'app_notes', icon: 'notes', href: '/notes' }],
  // The attachment download policy is part of the sync settings form.
  settings: [{ id: 'attachments', title: 'settings_att_section', group: 'settings_data', order: 50, into: 'sync', component: AttachmentPolicyBlock }],
  overlays: [{ id: 'more', component: NotesMoreOverlay, place: 'window' }],
  bar: notesBar,
  split: splitOf,
  // Home's search bar opens the notes' search.
  search: '/search',
  labelField: EntityLabelField,
};
