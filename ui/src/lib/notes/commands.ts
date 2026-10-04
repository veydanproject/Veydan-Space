// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/** The notes' commands for the palette (`ModuleDef.commands`), registered once by the shell. */

import { get } from 'svelte/store';
import { goto } from '$app/navigation';
import { api } from '$lib/notes/api';
import { t } from '$lib/core/i18n';
import { commands, type Command } from '$lib/core/commands';
import { notesStore } from '$lib/notes/store/notes.svelte';
import { templateNotes } from '$lib/notes/filter';

const tr = () => get(t);

async function toNotes(): Promise<void> {
  await notesStore.ensureLoaded();
  await goto('/notes');
}

let registered = false;

export function registerNotesCommands(): void {
  // The shell may remount (HMR, window reopen); providers must not stack up
  if (registered) return;
  registered = true;
  // Provider rather than static registration so titles follow the current locale
  commands.addProvider(() => [
    {
      id: 'notes.create',
      title: tr()('cmd_notes_create'),
      keywords: 'new note',
      icon: 'plus',
      group: tr()('cmd_group_notes'),
      run: async () => {
        await toNotes();
        notesStore.uiRequest = { kind: 'create' };
      },
    },
    {
      id: 'notes.search',
      title: tr()('cmd_notes_search'),
      keywords: 'find',
      icon: 'search',
      group: tr()('cmd_group_notes'),
      run: async () => {
        await toNotes();
        notesStore.uiRequest = { kind: 'search' };
      },
    },
    {
      id: 'notes.insertLink',
      title: tr()('cmd_notes_insert_link'),
      keywords: 'wiki [[',
      icon: 'link',
      group: tr()('cmd_group_notes'),
      when: () => notesStore.activeNote !== null && !notesStore.activeNote.deleted,
      run: () => {
        notesStore.uiRequest = { kind: 'insertLink' };
      },
    },
    {
      id: 'notes.quickCapture',
      title: tr()('cmd_notes_quick_capture'),
      icon: 'zap',
      group: tr()('cmd_group_notes'),
      run: () => api.notes.openQuickCapture(),
    },
  ]);

  // One command per smart view
  commands.addProvider(() =>
    notesStore.smartViews.map((v): Command => ({
      id: `notes.smartView.${v.id}`,
      title: tr()('cmd_notes_smart_view', { name: v.name }),
      icon: 'filter',
      group: tr()('cmd_group_notes'),
      run: async () => {
        await toNotes();
        notesStore.uiRequest = { kind: 'filter', filter: { type: 'smart', id: v.id } };
      },
    })),
  );

  // New note from each template
  commands.addProvider(() =>
    templateNotes(notesStore.list, notesStore.folders).map((tpl): Command => ({
      id: `notes.fromTemplate.${tpl.id}`,
      title: tr()('cmd_notes_from_template', { name: tpl.title }),
      icon: 'file-text',
      group: tr()('cmd_group_notes'),
      run: async () => {
        await toNotes();
        const note = await notesStore.createNote({ title: tpl.title, template_id: tpl.id });
        notesStore.openRequestId = note.id;
      },
    })),
  );

  // Recently edited notes
  commands.addProvider(() =>
    [...notesStore.list]
      .filter((n) => !n.deleted && !n.archived)
      .sort((a, b) => b.updated_at.localeCompare(a.updated_at))
      .slice(0, 12)
      .map((n): Command => ({
        id: `notes.open.${n.id}`,
        title: tr()('cmd_notes_open', { title: n.title || tr()('notes_untitled') }),
        keywords: n.tags.map((x) => x.name).join(' '),
        icon: 'file-text',
        group: tr()('cmd_group_notes'),
        run: async () => {
          await toNotes();
          notesStore.openRequestId = n.id;
        },
      })),
  );
}
