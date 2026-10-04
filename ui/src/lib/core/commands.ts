// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { get } from 'svelte/store';
import { directory } from './directory';
import { t } from './i18n';
import type { ModuleId } from './module';
import { registry } from './registry';

/**
 * Global command registry behind the command palette. Static commands are
 * registered once; providers return commands derived from data (one per
 * SSH connection, smart view, ...) and run each time the palette lists them.
 * What a module registers belongs to it, and is listed while the module is
 * switched on (platform-spec 12).
 */

export interface Command {
  /** Stable id, e.g. `notes.create` or `ssh.connect.{id}` */
  id: string;
  title: string;
  /** Extra words the search should match */
  keywords?: string;
  icon?: string;
  /** Section shown in the palette */
  group: string;
  /** Hidden when false */
  when?: () => boolean;
  /**
   * An action of one entity of the catalog (`Copy code: GitHub`): there are
   * as many as the user has entities, so the palette lists only a few of each
   * group until something is typed.
   */
  entity?: boolean;
  run: () => void | Promise<void>;
}

export type CommandProvider = () => Command[];

class CommandRegistry {
  private static_ = new Map<string, { command: Command; owner?: ModuleId }>();
  private providers: { provider: CommandProvider; owner?: ModuleId }[] = [];
  /** The module whose registration runs now. */
  private owner: ModuleId | undefined;

  /** Runs a module's registration: what it registers is the module's. */
  as(owner: ModuleId, register: () => void): void {
    this.owner = owner;
    try {
      register();
    } finally {
      this.owner = undefined;
    }
  }

  register(...cmds: Command[]): void {
    for (const c of cmds) this.static_.set(c.id, { command: c, owner: this.owner });
  }

  addProvider(p: CommandProvider): void {
    // A shell that mounts again registers the same provider again: once is enough.
    if (this.providers.some((x) => x.provider === p)) return;
    this.providers.push({ provider: p, owner: this.owner });
  }

  /** Every currently available command; a later duplicate id is dropped. */
  all(): Command[] {
    const on = (owner?: ModuleId) => owner === undefined || registry.enabled(owner);
    const seen = new Map<string, Command>();
    for (const { command, owner } of this.static_.values()) if (on(owner)) seen.set(command.id, command);
    for (const { provider, owner } of this.providers) {
      if (!on(owner)) continue;
      for (const c of provider()) if (!seen.has(c.id)) seen.set(c.id, c);
    }
    return [...seen.values()].filter((c) => c.when?.() ?? true);
  }

  /** Commands whose title or keywords contain every word of `query`. */
  search(query: string, max = 40): Command[] {
    const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
    const all = this.all();
    if (words.length === 0) {
      // Nothing typed: what the app and its modules can do first, then a few
      // entities of each kind; every entity is found by typing.
      const perGroup = new Map<string, number>();
      const sample = all.filter((c) => {
        if (!c.entity) return false;
        const n = perGroup.get(c.group) ?? 0;
        perGroup.set(c.group, n + 1);
        return n < EMPTY_QUERY_ENTITIES;
      });
      return groupedInOrder([...all.filter((c) => !c.entity), ...sample]).slice(0, max);
    }
    const score = (c: Command): number => {
      const hay = `${c.title} ${c.keywords ?? ''} ${c.group}`.toLowerCase();
      if (!words.every((w) => hay.includes(w))) return -1;
      return c.title.toLowerCase().startsWith(words[0]) ? 0 : 1;
    };
    return all
      .map((c) => ({ c, s: score(c) }))
      .filter((x) => x.s >= 0)
      .sort((a, b) => a.s - b.s || a.c.title.localeCompare(b.c.title))
      .slice(0, max)
      .map((x) => x.c);
  }
}

/**
 * The list with each group's rows together, the groups in the order they
 * first appear and the rows of a group in their order: the palette prints a
 * group's heading once (a module's commands and its entities' actions share
 * a group, and the entities came after every command).
 */
export function groupedInOrder(list: Command[]): Command[] {
  const rank = new Map<string, number>();
  for (const c of list) if (!rank.has(c.group)) rank.set(c.group, rank.size);
  return list
    .map((c, i) => ({ c, i }))
    .sort((a, b) => rank.get(a.c.group)! - rank.get(b.c.group)! || a.i - b.i)
    .map((x) => x.c);
}

/** How many entity actions of one group the palette lists before anything is typed. */
const EMPTY_QUERY_ENTITIES = 3;

export const commands = new CommandRegistry();

let directoryRegistered = false;

/**
 * One command per action of every entity the catalog knows — `Connect:
 * Production SG`, `Check: SG Proxy`, … — whichever module owns the kind.
 * Registered once by the desktop shell; a provider, so titles follow the locale.
 */
export function registerDirectoryCommands(): void {
  if (directoryRegistered) return;
  directoryRegistered = true;
  commands.addProvider(() => {
    const tr = get(t);
    const out: Command[] = [];
    for (const def of directory.kinds) {
      for (const entity of def.list()) {
        for (const action of def.actions) {
          out.push({
            id: `${def.kind}.${action.id}.${entity.id}`,
            title: `${tr(action.label)}: ${entity.name}`,
            keywords: `${def.kind} ${entity.subtitle}`,
            icon: action.icon,
            group: tr(def.label),
            entity: true,
            run: () => action.run(entity.id),
          });
        }
      }
    }
    return out;
  });
}
