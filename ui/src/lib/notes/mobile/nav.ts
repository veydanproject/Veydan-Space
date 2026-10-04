// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The notes' own bottom bar on the phone (`ModuleDef.bar`): notes, new, tags, more.

import type { NavItem } from '$lib/core/module';
import { notesMobileUi } from './ui.svelte';

const NEW_NOTE_KINDS = new Set(['folder', 'workspace', 'profile']);

/** Keep the open folder, workspace, or profile on the new-note route. */
export function newNoteHref(url: URL): string {
  if (url.pathname !== '/notes') return '/notes/new';
  const kind = url.searchParams.get('kind') ?? '';
  const id = url.searchParams.get('id') ?? '';
  if (!id || !NEW_NOTE_KINDS.has(kind)) return '/notes/new';
  return `/notes/new?${new URLSearchParams({ kind, id })}`;
}

export function notesNav(newHref = '/notes/new'): NavItem[] {
  return [
    { id: 'notes', title: 'app_notes', icon: 'file-text', href: '/notes' },
    { id: 'new', title: 'nav_new', icon: 'file-plus', href: newHref },
    { id: 'tags', title: 'notes_tags', icon: 'tag', href: '/notes/tags' },
    { id: 'more', title: 'nav_more', icon: 'more-horizontal', onclick: () => { notesMobileUi.moreOpen = true; } },
  ];
}

/** The bar for a path of the notes: none in the editor, the notes' bar on the list, search and tags. */
export function notesBar(url: URL): NavItem[] | null | undefined {
  const { pathname } = url;
  if (/^\/notes\/(?!tags$)[^/]+/.test(pathname)) return null;
  if (pathname.startsWith('/notes') || pathname.startsWith('/search')) return notesNav(newNoteHref(url));
  return undefined;
}
