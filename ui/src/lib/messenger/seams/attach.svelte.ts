// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What a composer on a phone can attach (platform-spec 6.3). The sheet of
// the paperclip knows no item: it draws the tabs this registry holds for
// the composer it was opened from. The messenger gives its own items
// («Галерея», «Файл», «Из буфера») through `provideAttachAction`, as a
// module of the product gives its own (a file from the cloud, a note); a
// product without the module has no such tab. A new kind of content is a
// new `AttachResultType`: a composer that does not accept it never shows
// the item.

import type { Component, Snippet } from 'svelte';
import type { TranslationKey } from '$lib/core/i18n';

/** What an item hands to the composer: `files` today; `location`, `contact`, `poll`… when a composer takes them. */
export type AttachResultType = 'files' | (string & {});

/** Where the sheet was opened. */
export type AttachSurface = 'dm' | 'group' | 'channel' | 'thread' | 'post';

/** What the composer the sheet was opened from says of itself. */
export interface AttachContext {
  surface: AttachSurface;
  chatId: string;
  /** The result types this composer can take (today only `files`). */
  accepts: AttachResultType[];
  limits: { maxBytes: number; maxCount: number };
}

export type AttachResult =
  /**
   * Paths or `content://` addresses for `messengerApi.media.importPicked`; `original`: without
   * compression; `asFile`: they go as files, as they are (the item «Файл»: a HEIC is not made a JPEG).
   */
  | { type: 'files'; paths: string[]; original: boolean; asFile?: boolean }
  | { type: string; data: unknown };

/** What the panel of a tab is given. */
export interface AttachPanelProps {
  ctx: AttachContext;
  /** Hand the result to the composer; the sheet closes. */
  done: (result: AttachResult | null) => void;
  /**
   * Put a bar of the panel's own in place of the strip of items while it
   * holds a choice («Добавить (N)»); `null` gives the strip back. The bar
   * goes with the panel.
   */
  bar?: (bar: Snippet | null) => void;
}

export interface AttachAction {
  /** `gallery`, `file`, `location`… */
  id: string;
  /** The place in the strip of tabs, ascending. */
  order: number;
  icon: string;
  label: TranslationKey;
  badge?: 'new';
  /** The item is shown only where the composer accepts this type. */
  produces: AttachResultType;
  /** The module, the surface, the platform: `false` hides the item. */
  available?: (ctx: AttachContext) => boolean;
  /** A tab with content (the gallery, a map). */
  panel?: Component<AttachPanelProps>;
  /** Or an action at once (a file): the sheet closes and runs it; `null`: nothing chosen. */
  run?: (ctx: AttachContext) => Promise<AttachResult | null>;
}

/**
 * One giving of an item. Every `provideAttachAction` makes a new one, even
 * for an action given before: a result is taken for the registration it was
 * started from, so a module that stops and gives the same object again does
 * not bring the answers of the old one back.
 */
export interface AttachRegistration {
  readonly action: AttachAction;
}

class AttachSeam {
  // Raw: the registrations are compared by identity.
  entries = $state.raw<AttachRegistration[]>([]);

  /** The items given now. */
  get all(): AttachAction[] {
    return this.entries.map((r) => r.action);
  }

  /** Whether this very registration was neither taken back nor replaced; read at once, not after Svelte's effects. */
  alive(r: AttachRegistration): boolean {
    return this.entries.includes(r);
  }

  /** The items for a composer, in the order of the strip. */
  for(ctx: AttachContext): AttachRegistration[] {
    return this.entries
      .filter(({ action: a }) => ctx.accepts.includes(a.produces) && a.available?.(ctx) !== false)
      .toSorted((a, b) => a.action.order - b.action.order);
  }
}

export const attachActions = new AttachSeam();

/** A module gives an item at its start; returns the way to take this registration back, once and for good, at its stop. */
export function provideAttachAction(action: AttachAction): () => void {
  const reg: AttachRegistration = Object.freeze({ action });
  attachActions.entries = [...attachActions.entries.filter((r) => r.action.id !== action.id), reg];
  return () => {
    attachActions.entries = attachActions.entries.filter((r) => r !== reg);
  };
}
