// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The notes module as the desktop shell sees it (platform-spec 11.1).

import type { ModuleDef } from '$lib/core/module';
import { common } from './common';
import { registerNotesCommands } from '$lib/notes/commands';
import { api } from '$lib/notes/api';
import { t } from '$lib/core/i18n';
import { get } from 'svelte/store';
import NotesDirCard from '$lib/notes/desktop/NotesDirCard.svelte';
import NoteAttachmentPolicy from '$lib/notes/components/NoteAttachmentPolicy.svelte';
import ProfileNotesTab from '$lib/notes/desktop/ProfileNotesTab.svelte';
import WorkspaceNotesDrawer from '$lib/notes/desktop/WorkspaceNotesDrawer.svelte';
import ProxyNotes from '$lib/notes/desktop/ProxyNotes.svelte';
import SshNotes from '$lib/notes/desktop/SshNotes.svelte';
import { NOTES_WINDOWS } from '$lib/notes/window';

export const module: ModuleDef = {
  ...common,
  nav: [{ id: 'notes', title: 'nav_notes', icon: 'file-text', href: '/notes' }],
  settings: [
    { id: 'notes', title: 'settings_notes_section', group: 'settings_group_notes', order: 30, hint: 'settings_notes_folder_hint', component: NotesDirCard },
    { id: 'attachments', title: 'settings_att_section', group: 'settings_group_notes', order: 31, hint: 'settings_att_hint', component: NoteAttachmentPolicy },
  ],
  commands: registerNotesCommands,
  tray: () => ({ section_notes: get(t)('nav_notes'), quick_capture: get(t)('tray_quick_capture') }),
  windows: NOTES_WINDOWS,
  // The three panes of the notes page use the whole content area.
  fullWidth: ['/notes'],
  // The notes in their own window; with client-side decorations the button sits in the title bar.
  tools: [{ id: 'window', title: 'notes_btn_open_window', icon: 'file-text', place: 'title', onclick: () => void api.notes.openWindow(get(t)('nav_notes')) }],
  views: [
    { id: 'notes', scope: 'profile', as: 'tab', title: 'panel_tab_notes', icon: 'file-text', order: 30, component: ProfileNotesTab },
    { id: 'notes', scope: 'workspace', as: 'drawer', title: 'notes_label', icon: 'file-text', order: 30, component: WorkspaceNotesDrawer },
    { id: 'notes', scope: 'proxy', as: 'inline', title: 'notes_label', icon: 'file-text', order: 10, component: ProxyNotes },
    { id: 'notes', scope: 'ssh', as: 'inline', title: 'notes_label', icon: 'file-text', order: 10, component: SshNotes },
  ],
};
