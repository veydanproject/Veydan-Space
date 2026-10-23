// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The feeds of the channels and the threads of their posts as this device
// has read them, kept while the app runs: a channel opened again shows what
// was read at once, where it was left, and reads its newest page behind it.
// A list is keyed by its channel and what it shows (a section of the feed,
// the comments of one post); a page read again is merged into it by id, an
// item that did not change keeps its object, so a post on the screen is not
// drawn again because a reaction came to another one.
//
// What I write is shown before it is read back: a post or a comment of
// mine stands in the list at once (`local:` until the command answers with
// its id), a reaction is toggled on the tap; each is marked as on its way
// until the runtime tells of it (`channel.message`, or `channel.outbox`
// `sent` and the `pending` of the item of a core that keeps an outbox), and
// a refusal takes it back.

import { withEarlier, withNewest } from './feed';
import type { FeedCursorView, FeedItemView, FeedPageView } from './api';

/** An item as the runtime gives it: `pending` while it waits in the outbox. */
export type FeedItem = FeedItemView;

/** What the outbox of the runtime tells of something of mine (`channel.outbox`). */
export interface OutboxEvent {
  community_id?: string;
  ref_id?: string;
  state?: 'queued' | 'sent' | 'failed' | string;
  error?: string;
}

/** One list as read. */
export interface FeedList {
  /** Oldest first, as shown. */
  items: FeedItem[];
  /** The cursor of the page before what was read; none: nothing earlier. */
  next?: FeedCursorView;
  /** Its first page was read: the list is shown, never «loading» again. */
  loaded: boolean;
  /** The newest page came back empty while the relays were asked for more. */
  asked: boolean;
  /** How far from the bottom it was left; `null`: at the bottom. */
  anchor: number | null;
}

/** Reads a page: the newest without a cursor, an earlier one with it. */
export type PageReader = (next?: FeedCursorView) => Promise<FeedPageView>;

/** The feed of a channel, of all I am in or of one section. */
export const feedKey = (communityId: string, scope: string | null = null) => `${communityId}|${scope ?? '*'}`;
/** The section a feed's key shows: `null` for all I am in. */
export function scopeOfKey(key: string): string | null {
  const s = key.split('|')[1];
  return s === undefined || s === '*' ? null : s;
}
/** The comments of a post. */
export const threadKey = (communityId: string, postId: string) => `${communityId}#${postId}`;
const channelOfKey = (key: string) => key.split(/[|#]/, 1)[0];

/** Lists kept; the least recently opened go first. */
export const MAX_LISTS = 24;
/** Items a list keeps once left: its newest, the rest read again when scrolled to. */
export const KEEP_ITEMS = 60;
/** Ids told by the runtime remembered, for a command that answers after its event. */
const HEARD_MAX = 500;

/** My reaction set (`true`) or taken back on an item, as a page shows it. */
export function withReaction<T extends Pick<FeedItemView, 'reactions'>>(item: T, emoji: string, set: boolean): T {
  const r = item.reactions.find((x) => x.emoji === emoji);
  if (set) {
    if (r?.mine) return item;
    const reactions = r ? item.reactions.map((x) => (x === r ? { ...x, count: x.count + 1, mine: true } : x)) : [...item.reactions, { emoji, count: 1, mine: true }];
    return { ...item, reactions };
  }
  if (!r?.mine) return item;
  return { ...item, reactions: item.reactions.map((x) => (x === r ? { ...x, count: x.count - 1, mine: false } : x)).filter((x) => x.count > 0) };
}

/** The same item, as the page would draw it. */
function same(a: FeedItem, b: FeedItem): boolean {
  return a === b || JSON.stringify(a) === JSON.stringify(b);
}

let localSeq = 0;
/** An id for an item of mine before the command answers with its own. */
export const localId = () => `local:${Date.now().toString(36)}-${(localSeq++).toString(36)}`;
export const isLocal = (id: string) => id.startsWith('local:');

class FeedStore {
  lists = $state<Record<string, FeedList>>({});
  /** Items of mine shown before a read brings them, by list. */
  local = $state<Record<string, FeedItem[]>>({});
  /** Ids a command answered with, until the runtime tells of them. */
  awaiting = $state<Record<string, true>>({});
  /** Ids the outbox could not send, with its reason. */
  failed = $state<Record<string, string>>({});
  /** Reactions toggled here while their command runs: `id\nemoji` → set. */
  reacting = $state<Record<string, boolean>>({});
  /** The newest date sent as read, by channel: the same is not sent twice. */
  readUpTo: Record<string, number> = {};

  private heard = new Set<string>();
  private running = new Map<string, Promise<void>>();
  private again = new Set<string>();
  private earlierRunning = new Set<string>();
  /** Keys by when they were last opened, the newest last. */
  private order: string[] = [];
  private open = new Set<string>();

  list(key: string): FeedList | null {
    return this.lists[key] ?? null;
  }

  /**
   * What a list shows: what was read with my reactions on their way over it,
   * then my items not read back yet. Items the overlay leaves alone keep
   * their objects.
   */
  shown(key: string): FeedItem[] {
    const read = this.lists[key]?.items ?? [];
    const toggles = Object.entries(this.reacting);
    const base = toggles.length ? read.map((x) => this.overlay(x, toggles)) : read;
    const mine = this.local[key];
    if (!mine?.length) return base;
    const ids = new Set(base.map((x) => x.id));
    return [...base, ...mine.filter((x) => !ids.has(x.id))];
  }

  private overlay(item: FeedItem, toggles: [string, boolean][]): FeedItem {
    let out = item;
    for (const [k, set] of toggles) {
      const [id, emoji] = k.split('\n');
      if (id === item.id) out = withReaction(out, emoji, set);
    }
    return out;
  }

  /** On its way: written here and not told of yet, or waiting in the outbox. */
  isPending(item: Pick<FeedItem, 'id'> & { pending?: boolean }): boolean {
    if (this.failed[item.id] !== undefined) return false;
    return item.pending === true || isLocal(item.id) || !!this.awaiting[item.id];
  }

  /** Why it did not go; `null` while it goes or went. */
  failure(id: string): string | null {
    return this.failed[id] ?? null;
  }

  // ── Reading ────────────────────────────────────────────────────────────

  /** The list is on the screen: it is not trimmed nor dropped meanwhile. */
  enter(key: string) {
    this.open.add(key);
    this.order = [...this.order.filter((k) => k !== key), key];
    this.evict();
  }

  /**
   * The list leaves the screen where it was (`anchor`: from the bottom,
   * `null` at it); it keeps its newest items only, the rest are read again.
   */
  leave(key: string, anchor: number | null) {
    this.open.delete(key);
    const l = this.lists[key];
    if (!l) return;
    const trimmed = l.items.length > KEEP_ITEMS && anchor === null;
    // Left as it was: nothing written, so no list read from the store is drawn again (a switch of sections).
    if (!trimmed && l.anchor === anchor) return;
    const items = trimmed ? l.items.slice(-KEEP_ITEMS) : l.items;
    const next = trimmed ? { created_at: items[0].created_at, id: items[0].id } : l.next;
    this.lists = { ...this.lists, [key]: { ...l, items, next, anchor } };
  }

  private evict() {
    while (this.order.length > MAX_LISTS) {
      const gone = this.order.find((k) => !this.open.has(k));
      if (!gone) return;
      this.order = this.order.filter((k) => k !== gone);
      const { [gone]: _l, ...rest } = this.lists;
      this.lists = rest;
    }
  }

  /**
   * The newest page read again and merged into the list. A read asked while
   * one runs is done once after it: a burst of messages is one more read,
   * never several at once.
   */
  refresh(key: string, read: PageReader): Promise<void> {
    const now = this.running.get(key);
    if (now) {
      this.again.add(key);
      return now;
    }
    const run = (async () => {
      try {
        do {
          this.again.delete(key);
          this.merge(key, await read());
        } while (this.again.has(key));
      } finally {
        this.running.delete(key);
        this.again.delete(key);
      }
    })();
    this.running.set(key, run);
    return run;
  }

  /** A newest page into the list: what it covers is replaced, what was read above it stays. */
  merge(key: string, page: FeedPageView) {
    const was = this.lists[key];
    const old = was?.items ?? [];
    const byId = new Map(old.map((x) => [x.id, x]));
    const fresh = (page.items as FeedItem[]).map((x) => {
      const o = byId.get(x.id);
      return o && same(o, x) ? o : x;
    });
    // What the runtime reads as nothing left in its outbox is not on its
    // way, whatever the events said before: a `queued` heard after the
    // `sent` of the same item (a post sealed again under a new key goes
    // out a second time) left the clock on a delivered post for good
    // (живой запуск 2026-10-23, a post with a picture on the desktop).
    const delivered = fresh.filter((x) => x.pending === false && this.awaiting[x.id]).map((x) => x.id);
    if (delivered.length) {
      const awaiting = { ...this.awaiting };
      for (const id of delivered) delete awaiting[id];
      this.awaiting = awaiting;
    }
    const merged = withNewest(old, fresh);
    const unchanged = was && merged.items.length === old.length && merged.items.every((x, i) => x === old[i]);
    if (!unchanged || !was.loaded || was.asked !== page.asked) {
      this.lists = {
        ...this.lists,
        [key]: {
          items: unchanged ? old : merged.items,
          next: merged.older ? was?.next : (page.next ?? undefined),
          loaded: true,
          asked: page.asked && merged.items.length === 0,
          anchor: was?.anchor ?? null,
        },
      };
    }
    this.settleLocal(key, fresh);
    if (!this.order.includes(key)) {
      this.order = [...this.order, key];
      this.evict();
    }
  }

  /** My items a read brought are shown as read; those still on their way stay. */
  private settleLocal(key: string, fresh: FeedItem[]) {
    const mine = this.local[key];
    if (!mine?.length) return;
    const ids = new Set(fresh.map((x) => x.id));
    const left = mine.filter((x) => !ids.has(x.id));
    if (left.length !== mine.length) this.local = { ...this.local, [key]: left };
  }

  /** A page from before what was read, above it. */
  async earlier(key: string, read: PageReader): Promise<boolean> {
    const l = this.lists[key];
    if (!l?.next || this.earlierRunning.has(key)) return false;
    this.earlierRunning.add(key);
    try {
      const page = await read(l.next);
      const now = this.lists[key];
      if (!now) return false;
      this.lists = { ...this.lists, [key]: { ...now, items: withEarlier(now.items, page.items), next: page.next ?? undefined } };
      return true;
    } finally {
      this.earlierRunning.delete(key);
    }
  }

  isLoadingEarlier(key: string): boolean {
    return this.earlierRunning.has(key);
  }

  /** The list read from its newest again (a section picked anew): what it showed goes. */
  forget(key: string) {
    const { [key]: _l, ...rest } = this.lists;
    this.lists = rest;
  }

  // ── Writing ────────────────────────────────────────────────────────────

  /** An item of mine shown at once, before its command answers. */
  addLocal(key: string, item: FeedItem) {
    this.local = { ...this.local, [key]: [...(this.local[key] ?? []), item] };
  }

  /** The command answered: the item has its id, and waits to be told of unless it was already. */
  settle(key: string, temp: string, id: string) {
    const mine = this.local[key] ?? [];
    this.local = { ...this.local, [key]: mine.map((x) => (x.id === temp ? { ...x, id } : x)) };
    if (!this.heard.has(id)) this.awaiting = { ...this.awaiting, [id]: true };
  }

  /** The command refused: the item goes. */
  dropLocal(key: string, temp: string) {
    const mine = this.local[key];
    if (mine) this.local = { ...this.local, [key]: mine.filter((x) => x.id !== temp) };
  }

  /** The runtime told of an item (`channel.message`): it is no longer on its way. */
  heardOf(id: string) {
    this.heard.add(id);
    if (this.heard.size > HEARD_MAX) this.heard.delete(this.heard.values().next().value as string);
    if (this.awaiting[id]) {
      const { [id]: _a, ...rest } = this.awaiting;
      this.awaiting = rest;
    }
    if (this.failed[id] !== undefined) {
      const { [id]: _f, ...rest } = this.failed;
      this.failed = rest;
    }
  }

  /** The outbox of the runtime on something of mine: waiting, gone, or given up with a reason. */
  outbox(ev: OutboxEvent) {
    const id = ev.ref_id;
    if (!id) return;
    if (ev.state === 'queued') {
      this.awaiting = { ...this.awaiting, [id]: true };
      if (this.failed[id] !== undefined) {
        const { [id]: _f, ...rest } = this.failed;
        this.failed = rest;
      }
    } else if (ev.state === 'sent') {
      this.heardOf(id);
    } else if (ev.state === 'failed') {
      const { [id]: _a, ...rest } = this.awaiting;
      this.awaiting = rest;
      this.failed = { ...this.failed, [id]: ev.error ?? '' };
    }
  }

  /** The comments of a post one more (or fewer) at once, in every list of its channel; a read says how many there are. */
  bumpComments(communityId: string, postId: string, by: number) {
    this.mapItem(communityId, postId, (x) => ({ ...x, comments: Math.max(0, x.comments + by) }));
  }

  /**
   * My reaction toggled on the tap: shown at once, kept while the command
   * runs and written into what was read once it went; taken back when it
   * is refused.
   */
  async react(communityId: string, item: Pick<FeedItemView, 'id' | 'reactions'>, emoji: string, send: (set: boolean) => Promise<unknown>) {
    const k = `${item.id}\n${emoji}`;
    const set = k in this.reacting ? !this.reacting[k] : !item.reactions.some((r) => r.emoji === emoji && r.mine);
    this.reacting = { ...this.reacting, [k]: set };
    try {
      await send(set);
      this.mapItem(communityId, item.id, (x) => withReaction(x, emoji, set));
    } finally {
      // A tap again meanwhile owns the overlay now.
      if (this.reacting[k] === set) {
        const { [k]: _r, ...rest } = this.reacting;
        this.reacting = rest;
      }
    }
  }

  private mapItem(communityId: string, id: string, fn: (x: FeedItem) => FeedItem) {
    let lists = this.lists;
    for (const [key, l] of Object.entries(this.lists)) {
      if (channelOfKey(key) !== communityId) continue;
      const i = l.items.findIndex((x) => x.id === id);
      if (i < 0) continue;
      const items = l.items.slice();
      items[i] = fn(items[i]);
      lists = { ...lists, [key]: { ...l, items } };
    }
    if (lists !== this.lists) this.lists = lists;
  }

  /** The item of a channel wherever it was read; `null` when nowhere. */
  find(communityId: string, id: string): FeedItem | null {
    for (const [key, l] of Object.entries(this.lists)) {
      if (channelOfKey(key) !== communityId) continue;
      const x = l.items.find((y) => y.id === id);
      if (x) return x;
    }
    return null;
  }

  /** Everything of a channel gone (forgotten here). */
  dropChannel(communityId: string) {
    const keep = (k: string) => channelOfKey(k) !== communityId;
    this.lists = Object.fromEntries(Object.entries(this.lists).filter(([k]) => keep(k)));
    this.local = Object.fromEntries(Object.entries(this.local).filter(([k]) => keep(k)));
    this.order = this.order.filter(keep);
    delete this.readUpTo[communityId];
  }

  reset() {
    this.lists = {};
    this.local = {};
    this.awaiting = {};
    this.failed = {};
    this.reacting = {};
    this.readUpTo = {};
    this.heard.clear();
    this.order = [];
    this.open.clear();
  }
}

export const feedStore = new FeedStore();
