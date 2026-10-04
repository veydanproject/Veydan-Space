// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * The one registry of modules, for both shells (platform-spec 11.1, 11.7).
 * It is filled from the virtual module of the build's platform —
 * `virtual:veydan-modules/platform` (`/desktop` or `/mobile`), which the Vite
 * plugin builds from products.json — so a module the product does not have
 * is not here and not in the bundle. The root layout loads it before it mounts a
 * shell; everything that renders inside a shell may read it synchronously.
 * Which modules are switched on comes from the shell (section 12,
 * `modulesStore`): reading it here makes what renders from the registry
 * follow a switch at once.
 */

import { redirect } from '@sveltejs/kit';
import { modulesStore } from './store/modules.svelte';
import type { EntityViewDef, ModuleDef, ModuleId, NavItem, OverlayDef, SettingsSection, ToolItem } from './module';

const DEFAULT_APP_KEY = 'm_default_app';

class Registry {
  private defs: ModuleDef[] = [];
  private loading: Promise<void> | null = null;
  private started = new Set<ModuleId>();
  /** A shell started the modules: a switch starts and stops them from then on. */
  private running = false;

  constructor() {
    modulesStore.follow(() => {
      if (this.running) void this.follow();
    });
  }

  /** Reads the platform's virtual module and the switches of the shell once. */
  load(): Promise<void> {
    this.loading ??= Promise.all([
      import('virtual:veydan-modules/platform'),
      modulesStore.load(),
    ]).then(([{ modules }]) => {
      this.defs = Object.values(modules).map((m) => m.module);
    });
    return this.loading;
  }

  /** Descriptions of the product's modules, in the order of products.json. */
  get modules(): readonly ModuleDef[] {
    return this.defs;
  }

  get(id: ModuleId): ModuleDef | undefined {
    return this.defs.find((m) => m.id === id);
  }

  has(id: ModuleId): boolean {
    return this.defs.some((m) => m.id === id);
  }

  /** Whether the module is in the product and switched on (`modules_enabled`, section 12). */
  enabled(id: ModuleId): boolean {
    return this.has(id) && modulesStore.isOn(id);
  }

  /**
   * The modules the Modules section lists: those with a switch of their own
   * that this platform has, in the order of the product. Services have none.
   */
  switchable(): ModuleDef[] {
    const known = modulesStore.switchable;
    return this.defs.filter((m) => !m.service && (known.length === 0 || known.includes(m.id)));
  }

  /** Modules that are in the product and switched on. */
  get active(): ModuleDef[] {
    return this.defs.filter((m) => this.enabled(m.id));
  }

  /**
   * The product's only module, services aside (Pass, Notes, Chat): `/` opens
   * it on both platforms (platform-spec 11.7). Undefined in Space.
   */
  get only(): ModuleDef | undefined {
    const own = this.defs.filter((m) => !m.service);
    return own.length === 1 ? own[0] : undefined;
  }

  /** The first route of a module that is a screen of its own (not `/`). */
  homeRoute(m: ModuleDef): string | undefined {
    return m.routes.find((r) => r !== '/');
  }

  /** Mobile: the screen Home's search bar opens, from the module that has one. */
  search(): string | undefined {
    return this.active.find((m) => m.search)?.search;
  }

  /** The module whose route prefix the path starts with. */
  forPath(pathname: string): ModuleDef | undefined {
    return this.defs.find((m) =>
      m.routes.some((r) => (r === '/' ? pathname === '/' : pathname === r || pathname.startsWith(r + '/'))),
    );
  }

  /** Navigation entries of the active modules, those currently visible. */
  nav(): NavItem[] {
    return this.active.flatMap((m) => m.nav).filter((n) => n.visible?.() ?? true);
  }

  /** The entry a saved default-app id names, if its module is active. */
  navItem(id: string): NavItem | undefined {
    return this.active.flatMap((m) => m.nav).find((n) => n.id === id);
  }

  /** The top bar's buttons; a quick-access one only where its module is not the whole product. */
  tools(): ToolItem[] {
    return this.active.flatMap((m) => (m.tools ?? []).filter((tool) => !(tool.quick && this.only === m)));
  }

  /** Cards of the settings pages of the modules that are on. */
  settings(): SettingsSection[] {
    return this.active.flatMap((m) => m.settings);
  }

  /** Overlays of one place, in their declared order (ties keep the registry order). */
  overlays(place: NonNullable<OverlayDef['place']>): (OverlayDef & { key: string })[] {
    return this.active
      .flatMap((m) =>
        (m.overlays ?? []).filter((o) => !(o.quick && this.only === m)).map((o) => ({ ...o, key: `${m.id}:${o.id}` })),
      )
      .filter((o) => (o.place ?? 'window') === place)
      .sort((a, b) => (a.order ?? 0) - (b.order ?? 0));
  }

  /** Views other modules show for an entity of `scope`, in their declared order. */
  views(scope: string, as: EntityViewDef['as']): EntityViewDef[] {
    return this.active
      .flatMap((m) => m.views ?? [])
      .filter((v) => v.scope === scope && v.as === as)
      .sort((a, b) => a.order - b.order);
  }

  /** Every reloader of the active modules, once each. */
  reloaders(): (() => Promise<unknown>)[] {
    return [...new Set(this.active.flatMap((m) => Object.values(m.reloaders ?? {})))];
  }

  /** Reloaders for one sync entity type. */
  reloadersFor(type: string): (() => Promise<unknown>)[] {
    return this.active.flatMap((m) => (m.reloaders?.[type] ? [m.reloaders[type]] : []));
  }

  /**
   * Desktop: the tray menu's labels of the product's modules (`ModuleDef.tray`),
   * on or off: the backend leaves out the entries of a module that is off, and
   * has the words ready when it is switched on.
   */
  trayLabels(): Record<string, string> {
    return Object.assign({}, ...this.defs.map((m) => m.tray?.() ?? {}));
  }

  /** Desktop: the title of a module's own window, by its label. */
  windowTitle(label: string): ModuleDef['title'] | undefined {
    return this.defs.flatMap((m) => m.windows ?? []).find((w) => w.label === label)?.title;
  }

  /** Desktop: whether the page at `pathname` takes the whole content area. */
  fullWidth(pathname: string): boolean {
    return this.active.some((m) => m.fullWidth?.some((p) => pathname === p || pathname.startsWith(p + '/')));
  }

  /** Mobile: the editor of labels and references, from the module that keeps the labels. */
  labelField(): ModuleDef['labelField'] {
    return this.active.find((m) => m.labelField)?.labelField;
  }

  /** Mobile: the bar of the module owning the path; `undefined` when none claims it. */
  bar(url: URL): NavItem[] | null | undefined {
    for (const m of this.active) {
      const items = m.bar?.(url);
      if (items !== undefined) return items;
    }
    return undefined;
  }

  /**
   * The module opened at `/` on the desktop (platform-spec 11.7): the one the
   * user chose, else the first that has a desktop home screen, else the first
   * module; never a service.
   */
  defaultModule(): ModuleDef | undefined {
    const chosen = this.navItem(loadDefaultApp());
    const owner = chosen?.href ? this.forPath(chosen.href) : undefined;
    return owner ?? this.active.find((m) => m.home) ?? this.active.find((m) => !m.service);
  }

  /** `start` of every active module not started yet (section 12); idempotent. */
  async startAll(): Promise<void> {
    this.running = true;
    await Promise.all(
      this.active.map(async (m) => {
        if (this.started.has(m.id)) return;
        this.started.add(m.id);
        try {
          await m.start?.();
        } catch (e) {
          console.error(`module ${m.id} failed to start`, e);
        }
      }),
    );
  }

  stopAll(): void {
    this.running = false;
    for (const m of this.defs) this.stop(m);
  }

  /** A switch changed: the modules switched off stop, those switched on start. */
  async follow(): Promise<void> {
    for (const m of this.defs) if (!this.enabled(m.id)) this.stop(m);
    await this.startAll();
  }

  private stop(m: ModuleDef) {
    if (!this.started.delete(m.id)) return;
    try {
      m.stop?.();
    } catch (e) {
      console.error(`module ${m.id} failed to stop`, e);
    }
  }
}

export const registry = new Registry();

/** Nav entry opened at launch instead of Home (mobile); empty means Home. */
export function loadDefaultApp(): string {
  if (typeof localStorage === 'undefined') return '';
  const id = localStorage.getItem(DEFAULT_APP_KEY) ?? '';
  return registry.navItem(id) ? id : '';
}

export function saveDefaultApp(id: string) {
  if (typeof localStorage === 'undefined') return;
  if (id) localStorage.setItem(DEFAULT_APP_KEY, id);
  else localStorage.removeItem(DEFAULT_APP_KEY);
}

/**
 * The guard every module route directory's thin +layout.ts calls. In a
 * product without the module the Vite plugin swaps that file for a stub that
 * redirects; here the module is present, and the guard sends a switched-off
 * module's routes to `/` (section 12). A page open when its module is
 * switched off is left by the shell (`leavesOff`).
 */
export async function guardModule(id: ModuleId): Promise<void> {
  await registry.load();
  if (!registry.enabled(id)) redirect(307, '/');
}

/**
 * Whether the page at `pathname` belongs to a module that is switched off,
 * and the shell is to go to `/`. `/` itself opens the default module, which
 * is on.
 */
export function leavesOff(pathname: string): boolean {
  if (pathname === '/') return false;
  const owner = registry.forPath(pathname);
  return owner !== undefined && !registry.enabled(owner.id);
}
