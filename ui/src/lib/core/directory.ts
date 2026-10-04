// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * The catalog of entities on the UI side (platform-spec 10.1): the owners of
 * the kinds come from `ModuleDef.entities` through the registry, and a
 * consumer asks by kind and id without knowing which module answers. In a
 * product without the owner a kind has no provider here, or only a fallback
 * (`EntityKindDef.fallback`) that lists the names the backend keeps in its
 * `labels` table; the name of a single reference comes from that table
 * through the shell's command `labels_resolve` (`foreign-labels`, 10.3),
 * whichever module shows the reference.
 */

import type { TranslationKey } from './i18n';
import { foreignLabels, hexColor } from './foreign-labels.svelte';
import type { EntityKindDef, EntitySummary } from './module';
import { registry } from './registry';

/** Icon and name of every kind a reference may point at, whether its owner is in the product or not. */
const KINDS: Record<string, { icon: string; label: TranslationKey }> = {
  workspace: { icon: 'layers', label: 'ctx_kind_workspace' },
  profile: { icon: 'globe', label: 'ctx_kind_profile' },
  proxy: { icon: 'shield', label: 'ctx_kind_proxy' },
  ssh: { icon: 'terminal', label: 'ctx_kind_ssh' },
  totp: { icon: 'key', label: 'ctx_kind_totp' },
  password: { icon: 'lock', label: 'ctx_kind_password' },
  note: { icon: 'file-text', label: 'ctx_kind_note' },
};

/**
 * A reference `kind:id` whose kind has no owner in this product (10.3): it is
 * shown with the kind's icon and the name the backend keeps in `labels`;
 * before sync brought that name, with the kind's name and a short id. It has
 * no actions, and a template placeholder of its fields is marked "available
 * in Veydan Space".
 */
export interface ForeignEntity {
  kind: string;
  id: string;
  icon: string;
  /** Name of the kind. */
  kindLabel: TranslationKey;
  /** The name from `labels`; null while there is none. */
  name: string | null;
  /**
   * The color the label keeps (a workspace's), a hex color only; null
   * without one, and while there is no name.
   */
  color: string | null;
  /** The first characters of the id, for a reference without a name. */
  shortId: string;
}

/** How many characters of an id stand for it where no name is known. */
const SHORT_ID = 8;

class Directory {
  /**
   * Kinds that have a provider in this product, in registry order: the owner,
   * or a fallback where no module owns the kind.
   */
  get kinds(): EntityKindDef[] {
    const all = registry.active.flatMap((m) => m.entities ?? []);
    const owned = new Set(all.filter((d) => !d.fallback).map((d) => d.kind));
    return all.filter((d, i) => !(d.fallback && (owned.has(d.kind) || all.findIndex((x) => x.kind === d.kind) < i)));
  }

  get(kind: string): EntityKindDef | undefined {
    return this.kinds.find((d) => d.kind === kind);
  }

  has(kind: string): boolean {
    return this.get(kind) !== undefined;
  }

  /** Whether a module of this product owns the kind (a fallback does not). */
  owned(kind: string): boolean {
    const def = this.get(kind);
    return def !== undefined && !def.fallback;
  }

  /** Every entity of a kind; empty without an owner. */
  list(kind: string): EntitySummary[] {
    return this.get(kind)?.list() ?? [];
  }

  summary(kind: string, id: string): EntitySummary | undefined {
    return this.get(kind)?.list().find((e) => e.id === id);
  }

  /** Entities of `kind` whose name or subtitle contains `query` (case-insensitive). */
  search(kind: string, query: string, max = 8): EntitySummary[] {
    const q = query.trim().toLowerCase();
    const all = this.list(kind);
    const hits = q ? all.filter((e) => `${e.name} ${e.subtitle}`.toLowerCase().includes(q)) : all;
    return hits.slice(0, max);
  }

  /** Open fields of one entity, for template placeholders. */
  fields(kind: string, id: string): Record<string, string> {
    return this.get(kind)?.fields?.(id) ?? {};
  }

  /** Icon and name of a kind; known for every kind a reference may point at, owned or not. */
  kind(kind: string): { icon: string; label: TranslationKey } | undefined {
    const def = this.get(kind);
    return def ? { icon: def.icon, label: def.label } : KINDS[kind];
  }

  /**
   * A reference to a kind without an owner here, as 10.3 shows it; undefined
   * when a module owns the kind (ask the owner) or it is not a kind of entity.
   * The name is the backend's (`labels`): the one a fallback already lists,
   * else the one `labels_resolve` gives — asked on the first read and
   * reactive, so the reference is shown by its kind and short id only until
   * the answer comes, and for good when there is no label.
   */
  foreign(kind: string, id: string): ForeignEntity | undefined {
    if (this.owned(kind) || !(kind in KINDS) || !id) return undefined;
    const { icon, label: kindLabel } = KINDS[kind];
    const listed = this.summary(kind, id);
    const name = listed?.name.trim() || foreignLabels.name(kind, id);
    // A fallback that lists the entity gives its color; else the label's, with its name.
    const color = listed?.name.trim() ? hexColor(listed.color) : foreignLabels.color(kind, id);
    return { kind, id, icon, kindLabel, name, color: name ? color : null, shortId: id.slice(0, SHORT_ID) };
  }

  /**
   * The name of the entity a link points at, where there is one: the
   * owner's, or the label of a kind without an owner here. Undefined for a
   * link nothing names — a list row leaves such a link out (its card shows
   * it by `name`).
   */
  linkName(kind: string, id: string): string | undefined {
    return this.summary(kind, id)?.name || this.foreign(kind, id)?.name || undefined;
  }

  /**
   * The color of the entity a link points at, where it has one: the
   * owner's, or for a kind without an owner here the color its label keeps
   * — a workspace of Space linked from Pass is shown in its color (10.3).
   * Only a hex color comes from a label (`hexColor`).
   */
  color(kind: string, id: string): string | undefined {
    if (this.owned(kind)) return this.summary(kind, id)?.color || undefined;
    return this.foreign(kind, id)?.color ?? undefined;
  }

  /**
   * The text a reference `kind:id` is shown with: the entity's name; for a
   * kind no module owns here, 10.3's (the label, else the kind's name and a
   * short id); for an owned entity that is gone, its id.
   */
  name(kind: string, id: string, tr: (key: TranslationKey) => string): string {
    const known = this.summary(kind, id)?.name;
    if (known) return known;
    const foreign = this.foreign(kind, id);
    return foreign ? foreignName(foreign, tr) : id;
  }

  /** Load every owner's store the cards and pickers read from. */
  ensureLoaded(): Promise<unknown> {
    return Promise.all(this.kinds.map((d) => d.ensureLoaded().catch(() => {})));
  }
}

export const directory = new Directory();

/** The text of a reference without an owner: its name, else the kind's name and the short id. */
export function foreignName(entity: ForeignEntity, tr: (key: TranslationKey) => string): string {
  return entity.name ?? `${tr(entity.kindLabel)} ${entity.shortId}`;
}
