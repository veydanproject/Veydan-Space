// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * What a module tells the shell about itself (platform-spec 11.1). The
 * types live in the core so the core never names a module: the set of
 * modules of a product comes from products.json through the virtual
 * modules, and the registry ($lib/core/registry) hands the descriptions to
 * both shells.
 *
 * A module describes what does not depend on the shell once, in
 * `entry/common.ts`; `entry/desktop.ts` and `entry/mobile.ts` spread that
 * description and add what refers to the components of their shell.
 */

import type { Component, Snippet } from 'svelte';
import type { TranslationKey } from './i18n';
import type { MobileKey } from './mobile/i18n';

/** The same id as in Rust and in products.json. */
export type ModuleId = string;

/**
 * A key of the merged dictionary: the core's and those of the product's
 * modules, on the desktop or layered over for the phone.
 */
export type Key = TranslationKey | MobileKey;

/** An entry of the side bar, the bottom navigation or the Home grid. */
export interface NavItem {
  id: string;
  title: Key;
  icon: string;
  /** Route to open; omit when `onclick` handles it. */
  href?: string;
  onclick?: () => void;
  /** Shown only while true (the messenger before its runtime started). Default: always. */
  visible?: () => boolean;
  /** Count on the entry (unread messages); nothing is shown for 0. */
  badge?: () => number;
}

/** A button of the desktop top bar's right side (TOTP, passwords, generator). */
export interface ToolItem {
  id: string;
  title: Key;
  icon: string;
  onclick: () => void;
  /** With client-side decorations the item goes to the title bar, next to Settings. */
  place?: 'title' | 'bar';
  /**
   * Quick access to the module's own screen from the rest of the app (Pass's
   * drawers in Space). A product whose only module this is has no rest of the
   * app: its window is that screen, and the shell leaves the item out.
   */
  quick?: boolean;
}

/**
 * A card of the settings page. Cards with the same `group` form one block of
 * the page's navigation; blocks are ordered by the smallest `order` in them,
 * between the core's own blocks.
 */
export interface SettingsSection {
  id: string;
  title: Key;
  group: Key;
  order: number;
  /** The card's body. */
  component: Component;
  /** One line under the title. */
  hint?: Key;
  /** Tag next to the title (beta). */
  badge?: Key;
  /** Shown only while true. Default: always. */
  visible?: () => boolean;
  /**
   * Rendered inside the core's card of this id instead of as a card of its
   * own: `sync` — a module's part of the sync settings form (mobile), whose
   * component exports `save()` for the form's Save.
   */
  into?: 'sync';
}

/** A component the shell mounts once, outside any page. */
export interface OverlayDef {
  id: string;
  component: Component;
  /**
   * `bottom`: a bar of the shell's bottom stack, in flow (the dock, the SSH
   * sessions); `content`: over the page, inside the frame (the terminal);
   * `window`: outside the frame, hidden while the app is locked (drawers,
   * banners). Default: `window`.
   */
  place?: 'bottom' | 'content' | 'window';
  /** Position among the overlays of the same place. */
  order?: number;
  /** Like `ToolItem.quick`: left out where the module is the product's only one. */
  quick?: boolean;
}

export type EntityStatus = 'ok' | 'bad' | 'unknown';

/** One entity of a kind, as chips, context cards and pickers show it. */
export interface EntitySummary {
  id: string;
  name: string;
  /** Short second line, e.g. `SOCKS5 · Singapore`. */
  subtitle: string;
  status: EntityStatus | null;
  color: string;
  /** Key `kind:id` of the entity this one belongs to (a profile's workspace). */
  parent?: string;
}

export interface EntityAction {
  id: string;
  label: TranslationKey;
  icon: string;
  run: (id: string) => Promise<void>;
}

/**
 * The owner of an entity kind, on the UI side of the catalog of entities
 * (platform-spec 10). A consumer asks `$lib/core/directory` by kind and id
 * and does not know which module answers.
 */
export interface EntityKindDef {
  kind: string;
  icon: string;
  /** Name of the kind (a key of the desktop dictionary: the kinds are shown there). */
  label: TranslationKey;
  color: string;
  ensureLoaded: () => Promise<void>;
  list: () => EntitySummary[];
  actions: EntityAction[];
  /** Open fields of one entity for the placeholders of a note template (`field` of 10.1). */
  fields?: (id: string) => Record<string, string> | undefined;
  /**
   * Ids of this kind's entities that carry a reference to the entity `key`
   * (`kind:id`): the passwords tagged `note:<id>`.
   */
  referring?: (key: string) => string[];
  /** Owners that let another module create an entity of the kind (a label). */
  create?: (name: string, color?: string) => Promise<EntitySummary>;
  /**
   * Entities of this kind can hold `kind:value` references to others (a
   * note's bindings): add or remove one. The owner refreshes its own store.
   */
  link?: {
    add: (id: string, reference: string) => Promise<void>;
    remove: (id: string, reference: string) => Promise<void>;
  };
  /**
   * The provider answers for the kind only where no module of the product
   * owns it: the notes list the workspaces and profiles by the names the
   * backend keeps (`labels`, 10.2) where the browser module is absent. A
   * fallback has no actions, and its references are shown as 10.3 says.
   */
  fallback?: boolean;
  /** Show the entity in its module's UI (the workspace page, the password drawer). */
  open?: (id: string) => void;
  /**
   * The address of the entity's screen, for a link the core renders (the
   * sync form's list of conflicting notes); `conflict` opens the entity's
   * sync conflict where its module shows one.
   */
  href?: (id: string, view?: 'conflict') => string;
  /** Open the entity's editor where its module keeps it (the proxy form). */
  edit?: (id: string) => void;
  /**
   * A small component showing one entity live where another module lists it
   * (the current TOTP code, the copy buttons of a password in a note's
   * context card). Props: `id`; `children` are the host's own controls,
   * rendered where the component wants them.
   */
  inline?: Component<{ id: string; children?: Snippet }>;
  /** A block under the entity's row where a list shows it (the TOTP codes of a password). */
  details?: Component<{ id: string }>;
}

/**
 * A piece of UI a module shows inside another module's entity screen: a
 * tab of the profile panel, a drawer behind a button of the workspace page,
 * a block inside a form. The host renders the views registered for its
 * `scope` and knows nothing about who provided them.
 */
export interface EntityViewDef {
  id: string;
  /** Kind of the entity the view is shown for: `profile`, `workspace`, `proxy`, `ssh`. */
  scope: string;
  as: 'tab' | 'drawer' | 'inline';
  /** Name of the view: the tab's text, the drawer button's tooltip. */
  title: Key;
  /** Text of a drawer's button when it differs from the title. */
  label?: Key;
  icon: string;
  /** Position among the views of the same scope and form. */
  order: number;
  /** Count shown on a tab; nothing for 0. */
  count?: (id: string) => number;
  /**
   * Props: `id` and `name` of the scope entity; `workspaceId` for a
   * profile; `open` (bindable) for a drawer.
   */
  component: Component<{ id: string; name?: string; workspaceId?: string; open?: boolean }, Record<string, never>, '' | 'open'>;
}

/** Props of the editor of labels and references on a record (`ModuleDef.labelField`). */
export interface LabelFieldProps {
  /** Free labels and `kind:id` references, as stored on the record. */
  tags: string[];
  /** Offer notes and store `note:id` in tags. */
  linkNotes?: boolean;
  /** Hide the add chip. Existing tags can still be removed. */
  allowAdd?: boolean;
  onchange: (tags: string[]) => void;
}

/** A command of the palette as a module gives it (`ModuleDef.palette`; `core/commands.ts`). */
export interface PaletteCommand {
  id: string;
  title: string;
  keywords?: string;
  icon?: string;
  group: string;
  when?: () => boolean;
  /** An action of one entity: only a few of a group are listed before anything is typed. */
  entity?: boolean;
  run: () => void | Promise<void>;
}

export interface ModuleDef {
  id: ModuleId;
  /** i18n key of the module's name and the name of its icon. */
  title: Key;
  icon: string;
  /** Route prefixes; the first is the module's home screen. */
  routes: string[];
  /** Entries of the side bar, the bottom navigation and the Home grid. */
  nav: NavItem[];
  /** Cards of the settings page. */
  settings: SettingsSection[];
  /** Registration of palette commands (desktop). */
  commands?: () => void;
  /**
   * Palette commands as data (desktop), for a module that does not import
   * the core's command registry (the messenger, 6.3): the shell lists them
   * while the module is switched on.
   */
  palette?: () => PaletteCommand[];
  /** Owners of entity kinds for the catalog (section 10). */
  entities?: EntityKindDef[];
  /** Sync entity type → refresh of the store that shows it. */
  reloaders?: Record<string, () => Promise<unknown>>;
  /**
   * Subscriptions and loading of the stores (section 12): when the shell
   * starts, and when the user switches the module on; idempotent.
   */
  start?: () => Promise<void>;
  /** When the user switches the module off, and when the shell goes. */
  stop?: () => void;
  /** Lines a module adds to a bug report: a version of what it ships with (Camoufox). */
  report?: () => Promise<string[]>;
  /**
   * The last resort of the lock form when the synced vault cannot be opened:
   * the owner of the key's data deletes it and the lock gets a new key under
   * the same secret (section 8; pass: `password_vault_reset`). Offered only
   * where a module provides it.
   */
  vaultReset?: (secret: string) => Promise<void>;
  /**
   * A paragraph the confirmation of that reset adds: what of this module goes
   * with the key (the messenger's key on this device).
   */
  vaultResetNote?: Key;

  // Stage 8 additions (platform-spec 18.1): what the shells needed beyond 11.1.

  /** Components the shell mounts once: drawers, banners, the terminal, the dock. */
  overlays?: OverlayDef[];
  /** Buttons of the desktop top bar. */
  tools?: ToolItem[];
  /** Views shown inside other modules' entity screens. */
  views?: EntityViewDef[];
  /** The desktop screen at `/` when the module is the default one. */
  home?: Component;
  /**
   * Mobile: the module's own bottom bar for a path of its routes, or `null`
   * for a screen that takes the whole height. `undefined` leaves the bar to
   * the shell (the root bar).
   */
  bar?: (url: URL) => NavItem[] | null | undefined;
  /**
   * Mobile: asked once at start, before the default screen opens. True when
   * the module took the user somewhere (the chat of a tapped notification).
   */
  resume?: () => Promise<boolean>;
  /**
   * Mobile: the editor of labels and references another module's forms show
   * on their records (passwords, TOTP). The notes keep the labels, so the
   * notes module provides it; without it a form has no label editor.
   */
  labelField?: Component<LabelFieldProps>;
  /**
   * Desktop: labels of the module's entries in the tray menu, in the active
   * locale, under the keys its Rust tray reads (`tray_set_labels`). The shell
   * adds its own (show, hide, quit, tooltip) and sends them again when the
   * language changes.
   */
  tray?: () => Record<string, string>;
  /**
   * Desktop: the module's own windows besides `main` (the notes window, quick
   * capture), by window label. Such a window gets the theme and its chrome
   * only; the title bar shows `title`.
   */
  windows?: { label: string; title: Key }[];
  /** Desktop: route prefixes whose pages take the whole content area (the notes' panes). */
  fullWidth?: string[];
  /**
   * Links the About card adds for this module: what it is built on (the
   * browser's Camoufox). A product without the module does not show them.
   */
  about?: { id: string; title: Key; url: string }[];
  /**
   * The module has demo data (its Rust side registers a `DemoPart`, 5.9):
   * "Load demo data" is offered only in a product where some module has.
   */
  demo?: boolean;
  /**
   * The module's data in the sync vault holds files (the notes' attachments,
   * the browser's profile files): Settings → Sync offers the large-file
   * transfer settings only in a product where some module has.
   */
  syncsFiles?: boolean;
  /**
   * The module's shortcuts in Settings → Hotkeys, as ids of
   * `core/keybindings.ts` groups (the notes' editor). The core's own groups
   * are always listed; a module's only where the module is switched on.
   */
  hotkeys?: string[];
  /**
   * A service of the product (Space's backup), not a module the user works
   * in: no navigation, not in the Modules section (section 12), never the
   * module opened at `/`, not counted when a product is told to have one
   * module.
   */
  service?: boolean;
  /**
   * Mobile: the screen Home's search bar opens (the notes' search). Without
   * one Home shows no search bar.
   */
  search?: string;
}
