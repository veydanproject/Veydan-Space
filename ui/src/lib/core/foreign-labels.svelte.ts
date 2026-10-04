// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * Names of the entities whose kind has no owner in this product
 * (platform-spec 10.3): the shell's command `labels_resolve` answers them
 * from the synced `labels` table. The catalog (`$lib/core/directory`) asks
 * here for every foreign reference it shows, so a password linked to a
 * profile of Space and a note bound to one take the name the same way.
 *
 * A name is asked once and kept; the references asked in one tick go in one
 * call. When sync brings labels, every kept name is asked again: a link
 * shown by its kind and short id gets its name, a renamed entity its new one.
 * The color of the label (a workspace's) comes with the name, and only as a
 * hex color (`hexColor`): it goes into a `style`.
 */

import { api, type LabelRef } from '$lib/core/api';

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** Emitted with the entity types a cycle of sync changed here. */
const SYNC_CHANGED = 'sync://data-changed';

/** The most references one call asks for: the command refuses more (`MAX_ITEMS` in crates/shell/src/commands/labels.rs). */
export const LABELS_BATCH = 1000;

const HEX_COLOR = /^#(?:[0-9a-f]{3}|[0-9a-f]{6})$/i;

/**
 * The color when it is one the app writes (`#rgb`, `#rrggbb`), as it is;
 * null for anything else. The backend checks it the same way (`hex_color` in
 * crates/shell/src/commands/labels.rs); a label comes from sync, so the UI
 * checks again before a color reaches a `style`.
 */
export function hexColor(value: unknown): string | null {
  return typeof value === 'string' && HEX_COLOR.test(value) ? value : null;
}

class ForeignLabels {
  /** `kind:id` → the name; null while the backend has none. */
  private names = $state<Record<string, string | null>>({});
  /** `kind:id` → the label's color; null without one. */
  private colors = $state<Record<string, string | null>>({});
  /** Every reference asked so far, for the refresh. */
  private asked = new Map<string, LabelRef>();
  private queue = new Map<string, LabelRef>();
  private listening = false;

  /**
   * The name of the entity, or null while none is known. Reading it in a
   * template or a `$derived` subscribes to the answer: an unknown reference
   * is asked for after the current tick, never during it.
   */
  name(kind: string, id: string): string | null {
    const key = `${kind}:${id}`;
    const known = this.names[key];
    if (known === undefined && !this.asked.has(key)) {
      this.asked.set(key, { kind, id });
      this.enqueue(key, { kind, id });
    }
    return known ?? null;
  }

  /**
   * The color of the entity's label (a workspace's), or null; asked with the
   * name, and reactive the same way.
   */
  color(kind: string, id: string): string | null {
    this.name(kind, id);
    return this.colors[`${kind}:${id}`] ?? null;
  }

  /** Ask again for everything asked so far. */
  refresh(): void {
    for (const [key, item] of this.asked) this.enqueue(key, item);
  }

  /** Drop every name (tests; a wipe of the data). */
  forget(): void {
    this.names = {};
    this.colors = {};
    this.asked.clear();
    this.queue.clear();
  }

  private enqueue(key: string, item: LabelRef): void {
    if (this.queue.size === 0) queueMicrotask(() => void this.flush());
    this.queue.set(key, item);
    this.listen();
  }

  private async flush(): Promise<void> {
    const items = [...this.queue.values()];
    this.queue.clear();
    if (items.length === 0) return;
    for (let start = 0; start < items.length; start += LABELS_BATCH) {
      try {
        for (const answer of await api.labels.resolve(items.slice(start, start + LABELS_BATCH))) {
          const key = `${answer.kind}:${answer.id}`;
          // Only what was asked: an answer cannot name another reference.
          if (!this.asked.has(key)) continue;
          const name = answer.name?.trim() || null;
          this.names[key] = name;
          // No name, no color: a nameless link is shown neutral, by its kind.
          this.colors[key] = name ? hexColor(answer.color) : null;
        }
      } catch {
        // No answer: the references stay without names and are asked again on the next refresh.
      }
    }
    for (const { kind, id } of items) {
      const key = `${kind}:${id}`;
      if (this.asked.has(key) && this.names[key] === undefined) this.names[key] = null;
    }
  }

  /** One subscription for the life of the page, made with the first question. */
  private listen(): void {
    if (this.listening || !isTauri) return;
    this.listening = true;
    void import('@tauri-apps/api/event')
      .then(({ listen }) =>
        listen<string[]>(SYNC_CHANGED, (e) => {
          if (e.payload.includes('label')) this.refresh();
        }),
      )
      .catch(() => {
        this.listening = false;
      });
  }
}

export const foreignLabels = new ForeignLabels();
