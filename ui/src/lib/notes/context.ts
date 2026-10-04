// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * What a note knows about the entities it is bound to, through the catalog
 * of entities (`$lib/core/directory`): the owners of the kinds are other
 * modules, and in a product without one the kind is simply absent here.
 */

import { get } from 'svelte/store';
import { directory, foreignName } from '$lib/core/directory';
import { t } from '$lib/core/i18n';
import type { EntityKindDef, EntitySummary } from '$lib/core/module';
import { bindingValue, isEntityBinding, isEntityKind, parseBinding, type EntityKind } from '$lib/core/bindings';

export type { EntityAction, EntityStatus, EntitySummary } from '$lib/core/module';

/** The kinds a note can be bound to that have an owner in this product. */
export function entityKinds(): EntityKindDef[] {
  return directory.kinds.filter((d) => isEntityKind(d.kind));
}

/**
 * Whether a note can be bound to an object here: a module of this product owns
 * at least one kind (Notes alone owns none; a fallback only names foreign ones).
 */
export function canBindEntities(): boolean {
  return entityKinds().some((d) => !d.fallback);
}

export function entityKind(kind: EntityKind): EntityKindDef | undefined {
  return directory.get(kind);
}

export function entitySummary(kind: EntityKind, id: string): EntitySummary | undefined {
  return directory.summary(kind, id);
}

/** Bindings, mentions, and the entities that refer to the note (passwords tagged `note:id`). */
export function contextKeys(bindings: string[], mentions: string[], noteId?: string | null): string[] {
  const keys = new Set<string>();
  for (const item of bindings) if (isEntityBinding(item)) keys.add(item);
  for (const item of mentions) if (isEntityBinding(item)) keys.add(item);
  if (noteId) {
    for (const def of entityKinds()) {
      for (const id of def.referring?.(`note:${noteId}`) ?? []) keys.add(`${def.kind}:${id}`);
    }
  }
  return [...keys];
}

/**
 * Entities behind a note's bindings, as the table's context column shows them;
 * a kind without an owner in this product by the backend's label (10.3).
 */
export function noteContextEntities(note: { bindings: string[] }): { name: string; icon: string; color: string }[] {
  const out: { name: string; icon: string; color: string }[] = [];
  const tr = get(t);
  for (const b of note.bindings) {
    const p = parseBinding(b);
    if (!p || !isEntityKind(p.kind)) continue;
    const def = directory.get(p.kind);
    const e = def?.list().find((x) => x.id === p.value);
    if (def && e) {
      out.push({ name: e.name, icon: def.icon, color: e.color });
      continue;
    }
    const foreign = directory.foreign(p.kind, p.value);
    if (foreign) out.push({ name: foreignName(foreign, tr), icon: foreign.icon, color: foreign.color ?? 'var(--text-2)' });
  }
  return out;
}

/** Entities of `kind` whose name or subtitle contains `query` (case-insensitive). */
export function searchEntities(kind: EntityKind, query: string, max = 8): EntitySummary[] {
  return directory.search(kind, query, max);
}

/**
 * Current value of a template placeholder for a note; '' when nothing is bound.
 * Mirrors `templates.rs` so a placeholder picked in a regular note can be expanded in place.
 */
export function resolvePlaceholder(name: string, title: string, bindings: string[]): string {
  const now = new Date();
  const pad = (n: number) => String(n).padStart(2, '0');
  const date = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
  const time = `${pad(now.getHours())}:${pad(now.getMinutes())}`;
  const id = (kind: EntityKind) => bindingValue(bindings, kind);
  // A kind without an owner here gives no values: its name and the mark instead (10.3).
  const foreign = (kind: EntityKind, v: string) => {
    const f = directory.foreign(kind, v);
    if (!f) return undefined;
    const tr = get(t);
    return `${foreignName(f, tr)} (${tr('ctx_available_in_space')})`;
  };
  const field = (kind: EntityKind, key: string) => {
    const v = id(kind);
    if (!v) return '';
    return foreign(kind, v) ?? directory.fields(kind, v)[key] ?? '';
  };
  const name_ = (kind: EntityKind) => {
    const v = id(kind);
    if (!v) return '';
    return foreign(kind, v) ?? entitySummary(kind, v)?.name ?? '';
  };
  switch (name) {
    case 'date': return date;
    case 'time': return time;
    case 'datetime': return `${date} ${time}`;
    case 'title': return title;
    case 'workspace': return name_('workspace');
    case 'profile': return name_('profile');
    case 'totp': return name_('totp');
    case 'password': return name_('password');
    case 'url': return bindingValue(bindings, 'url') ?? '';
    case 'domain': return bindingValue(bindings, 'domain') ?? '';
    case 'proxy': return field('proxy', 'name');
    case 'proxy_type': return field('proxy', 'type');
    case 'proxy_host': return field('proxy', 'host');
    case 'proxy_port': return field('proxy', 'port');
    case 'proxy_region': return field('proxy', 'region');
    case 'ssh': return field('ssh', 'name');
    case 'ssh_host': return field('ssh', 'host');
    case 'ssh_port': return field('ssh', 'port');
    case 'ssh_user': return field('ssh', 'user');
    default: return '';
  }
}

/** Load every entity store the cards and pickers read from. */
export function ensureEntitiesLoaded(): Promise<unknown> {
  return directory.ensureLoaded();
}
