// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The heading of a notes list: what the sidebar selected, or the search. The
// Notes page and the workspace's Notes drawer show the same heading.

import { directory } from '$lib/core/directory';
import { isEntityKind } from '$lib/core/bindings';
import type { TranslationKey } from '$lib/core/i18n';
import type { ActiveFilter } from '$lib/notes/filter';
import type { NoteFolder, NoteSmartView } from '$lib/notes/types';

type Tr = (key: TranslationKey, vars?: Record<string, string>) => string;

export function notesListTitle(
  f: ActiveFilter,
  search: string,
  ctx: { t: Tr; folders: NoteFolder[]; smartViews: NoteSmartView[] },
): string {
  const { t } = ctx;
  if (search.trim()) return t('notes_list_search');
  const id = f.id ?? '';
  if (isEntityKind(f.type) && id) return directory.name(f.type, id, t);
  switch (f.type) {
    case 'global': return t('notes_filter_global');
    case 'pinned': return t('notes_filter_pinned');
    case 'archived': return t('notes_filter_archived');
    case 'trash': return t('notes_filter_trash');
    case 'folder': return ctx.folders.find((x) => x.id === id)?.name ?? id;
    case 'smart': return ctx.smartViews.find((v) => v.id === id)?.name ?? t('notes_filter_all');
    case 'tag':
    case 'tag-group':
    case 'domain': return id || t('notes_filter_all');
    default: return t('notes_filter_all');
  }
}
