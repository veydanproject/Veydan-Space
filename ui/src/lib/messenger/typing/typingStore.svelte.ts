// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Who is writing right now, as the runtime last heard it: the peer of a
// direct chat (`dm.typing`), members of a group (`group.typing`), people
// writing a comment under a post of a channel (`channel.typing`, handed on
// by the channels' store). Only bots say it, again every few seconds while
// they write, each time with until when; nobody says it stopped. So typing
// is a deadline, not a flag: a clock of 1 s, running only while somebody
// types, drops what ran out, and a live message of the same author ends
// it at once.

import type { MessengerUiEvent } from '../api';

const TICK_MS = 1000;
/** A deadline further ahead is cut to this: a sender cannot pin the line for good. */
const MAX_AHEAD_SECS = 15;

/** Where somebody types: a direct chat, a group, or under a post of a channel. */
export type TypingScope = { chat: string } | { group: string } | { post: string };

/** Who types in a direct chat, and until when (unix seconds). */
export interface DmTyping {
  peer: string;
  until: number;
}

/** Hex key → until when (unix seconds). */
type Typers = Record<string, number>;

const nowSecs = () => Date.now() / 1000;
const str = (v: unknown): string | null => (typeof v === 'string' && v ? v : null);

/** `until` cut to the longest believed; `null` when it is not a number. */
function deadline(v: unknown): number | null {
  if (typeof v !== 'number' || !Number.isFinite(v)) return null;
  return Math.min(v, nowSecs() + MAX_AHEAD_SECS);
}

/** The typers of `id` with `who` until `until`; a deadline already passed takes `who` out. */
function put(all: Record<string, Typers>, id: string, who: string, until: number): Record<string, Typers> {
  if (until <= nowSecs()) return drop(all, id, who);
  // A repeat keeps its place: the first to type is named first.
  return { ...all, [id]: { ...all[id], [who]: until } };
}

function drop(all: Record<string, Typers>, id: string, who: string): Record<string, Typers> {
  const typers = all[id];
  if (!typers || !(who in typers)) return all;
  const { [who]: _gone, ...rest } = typers;
  if (Object.keys(rest).length) return { ...all, [id]: rest };
  const { [id]: _empty, ...others } = all;
  return others;
}

/** The same without what ran out by `now`; the very object when nothing did. */
function prune(all: Record<string, Typers>, now: number): Record<string, Typers> {
  let out = all;
  for (const [id, typers] of Object.entries(all)) {
    for (const [who, until] of Object.entries(typers)) if (until <= now) out = drop(out, id, who);
  }
  return out;
}

class TypingStore {
  /** A direct chat's id → its peer typing. */
  chats = $state<Record<string, DmTyping>>({});
  /** A group's id → its members typing. */
  groups = $state<Record<string, Typers>>({});
  /** A post's id → who writes a comment under it. */
  posts = $state<Record<string, Typers>>({});
  private timer: ReturnType<typeof setInterval> | null = null;

  /** Runtime event → state. Called by the module store, and by the channels' store for `channel.typing`. */
  handleEvent(ev: MessengerUiEvent) {
    const p = (ev.payload ?? {}) as Record<string, unknown>;
    switch (ev.name) {
      case 'dm.typing': {
        const chat = str(p.chat_id);
        const peer = str(p.peer);
        const until = deadline(p.until);
        if (!chat || !peer || until === null) return;
        if (until <= nowSecs()) this.clear({ chat }, peer);
        else this.chats = { ...this.chats, [chat]: { peer, until } };
        break;
      }
      case 'group.typing': {
        const group = str(p.group_id);
        const member = str(p.member);
        const until = deadline(p.until);
        if (!group || !member || until === null) return;
        this.groups = put(this.groups, group, member, until);
        break;
      }
      case 'channel.typing': {
        const post = str(p.post_id);
        const author = str(p.author);
        const until = deadline(p.until);
        if (!post || !author || until === null) return;
        this.posts = put(this.posts, post, author, until);
        break;
      }
      // A message heard live ends its author's typing; one from the
      // history, or one of mine, ends nothing. A group's messages come in
      // the chat `group:<id>`. The event of a comment in a channel does not
      // name who wrote it, so the page of its thread ends the typing
      // (ThreadView).
      case 'dm.message': {
        if (p.historical === true) return;
        const m = p.message as { chat_id?: string; direction?: string; sender_pubkey?: string } | undefined;
        const chat = str(p.chat_id) ?? str(m?.chat_id);
        const sender = str(m?.sender_pubkey);
        if (!chat || !sender || m?.direction !== 'in') return;
        this.clear(chat.startsWith('group:') ? { group: chat.slice(6) } : { chat }, sender);
        return;
      }
      default:
        return;
    }
    this.tick();
  }

  /** Whether the peer of a direct chat types. */
  inChat(chatId: string): boolean {
    return chatId in this.chats;
  }

  /** The members of a group who type (hex), the first to start first. */
  inGroup(groupId: string): string[] {
    return Object.keys(this.groups[groupId] ?? {});
  }

  /** Who writes a comment under a post (hex), the first to start first. */
  underPost(postId: string): string[] {
    return Object.keys(this.posts[postId] ?? {});
  }

  /** `author` stopped typing there: their message came. */
  clear(scope: TypingScope, author: string) {
    if ('chat' in scope) {
      if (this.chats[scope.chat]?.peer === author) {
        const { [scope.chat]: _gone, ...rest } = this.chats;
        this.chats = rest;
      }
    } else if ('group' in scope) this.groups = drop(this.groups, scope.group, author);
    else this.posts = drop(this.posts, scope.post, author);
    this.tick();
  }

  /** The clock runs while somebody types, and only then. */
  private tick() {
    const idle = !Object.keys(this.chats).length && !Object.keys(this.groups).length && !Object.keys(this.posts).length;
    if (idle && this.timer) {
      clearInterval(this.timer);
      this.timer = null;
    } else if (!idle && !this.timer) {
      this.timer = setInterval(() => this.expire(), TICK_MS);
    }
  }

  /** Drops what ran out; a map where nothing did keeps its object, so nothing that reads it is drawn again. */
  private expire() {
    const now = nowSecs();
    if (Object.values(this.chats).some((c) => c.until <= now)) {
      this.chats = Object.fromEntries(Object.entries(this.chats).filter(([, c]) => c.until > now));
    }
    const groups = prune(this.groups, now);
    if (groups !== this.groups) this.groups = groups;
    const posts = prune(this.posts, now);
    if (posts !== this.posts) this.posts = posts;
    this.tick();
  }

  reset() {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
    this.chats = {};
    this.groups = {};
    this.posts = {};
  }
}

export const typingStore = new TypingStore();
