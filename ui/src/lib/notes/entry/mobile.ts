// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The notes module as the mobile shell sees it (platform-spec 11.1).

import type { ModuleDef } from '$lib/core/module';
import { common } from './common';
import { notesBar } from '$lib/notes/mobile/nav';
import NotesMoreOverlay from '$lib/notes/mobile/NotesMoreOverlay.svelte';
import EntityLabelField from '$lib/notes/mobile/EntityLabelField.svelte';
import AttachmentPolicyBlock from '$lib/notes/mobile/AttachmentPolicyBlock.svelte';

export const module: ModuleDef = {
  ...common,
  nav: [{ id: 'notes', title: 'app_notes', icon: 'file-text', href: '/notes' }],
  // The attachment download policy is part of the sync settings form.
  settings: [{ id: 'attachments', title: 'settings_att_section', group: 'settings_data', order: 50, into: 'sync', component: AttachmentPolicyBlock }],
  overlays: [{ id: 'more', component: NotesMoreOverlay, place: 'window' }],
  bar: notesBar,
  // Home's search bar opens the notes' search.
  search: '/search',
  labelField: EntityLabelField,
};
