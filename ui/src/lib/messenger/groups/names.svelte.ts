// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Who is behind a key: my own name for a contact first, then what they
// call themselves, then a short form of the key. Profiles of people who
// are not contacts are read once and kept for the session; one this
// device has never seen is asked of the relays, once, and comes back as
// `profile.updated`; asked again when the ask failed or a relay is back.

import { contactLabel, messengerApi, profileLabel, type MessengerProfile } from '../api';
import { messengerStore } from '../store.svelte';

/** How long a profile asked of the relays is awaited before it counts as not found. */
const SEEK_MS = 15_000;

export function shortKey(pubkey: string): string {
  return pubkey.length > 12 ? `${pubkey.slice(0, 6)}…${pubkey.slice(-4)}` : pubkey;
}

class NameStore {
  private profiles = $state<Record<string, MessengerProfile | null>>({});
  private asked = new Set<string>();
  /** Asked of the relays: once is enough while the relays hold the subscription. */
  private sought = new Set<string>();
  /** Asked of the relays and not answered yet: not «not found» while so. */
  private seeking = $state<Record<string, true>>({});
  private turns = new Map<string, number>();

  isMe(pubkey: string): boolean {
    return !!messengerStore.identity && messengerStore.identity.pubkey === pubkey;
  }

  /** Name to show; never empty. Reading it asks for the profile once. */
  label(pubkey: string): string {
    if (this.isMe(pubkey)) return profileLabel(messengerStore.ownProfile) || shortKey(pubkey);
    const c = messengerStore.contacts.find((x) => x.pubkey === pubkey);
    if (c) return contactLabel(c);
    const p = this.profiles[pubkey];
    if (p === undefined) this.ask(pubkey);
    return profileLabel(p ?? null) || shortKey(pubkey);
  }

  /** What they say about themselves; `null` until it is known. */
  profile(pubkey: string): MessengerProfile | null {
    if (this.isMe(pubkey)) return messengerStore.ownProfile ?? null;
    const c = messengerStore.contacts.find((x) => x.pubkey === pubkey);
    if (c?.profile) return c.profile;
    const p = this.profiles[pubkey];
    if (p === undefined) this.ask(pubkey);
    return p ?? null;
  }

  /**
   * Was their profile asked for and never found? False while it is on its
   * way, and for a contact or me (the book and my own profile hold theirs).
   */
  missing(pubkey: string): boolean {
    if (this.isMe(pubkey) || messengerStore.contacts.find((x) => x.pubkey === pubkey)?.profile) return false;
    return this.profiles[pubkey] === null && !this.seeking[pubkey];
  }

  picture(pubkey: string): string | null {
    if (this.isMe(pubkey)) return messengerStore.ownProfile?.picture ?? null;
    const c = messengerStore.contacts.find((x) => x.pubkey === pubkey);
    return c?.profile?.picture ?? this.profiles[pubkey]?.picture ?? null;
  }

  private ask(pubkey: string) {
    if (this.asked.has(pubkey) || !/^[0-9a-f]{64}$/.test(pubkey)) return;
    this.asked.add(pubkey);
    // Outside the render that asked: state must not change while reading it.
    queueMicrotask(() => {
      messengerApi.profiles.get(pubkey)
        .then((p) => {
          this.profiles = { ...this.profiles, [pubkey]: p ?? null };
          if (!p) this.seek(pubkey);
        })
        .catch(() => { this.profiles = { ...this.profiles, [pubkey]: null }; });
    });
  }

  /**
   * Not in the cache (a stranger, who is neither a contact nor in a group
   * of mine, whose profile nothing else follows): asked of the relays once.
   */
  private seek(pubkey: string) {
    if (this.sought.has(pubkey)) return;
    this.sought.add(pubkey);
    this.seeking = { ...this.seeking, [pubkey]: true };
    // An earlier ask's end must not cut this one's wait short.
    const turn = (this.turns.get(pubkey) ?? 0) + 1;
    this.turns.set(pubkey, turn);
    const done = () => {
      if (this.turns.get(pubkey) !== turn || !this.seeking[pubkey]) return;
      const { [pubkey]: _, ...rest } = this.seeking;
      this.seeking = rest;
    };
    setTimeout(done, SEEK_MS);
    messengerApi.profiles.request(pubkey).catch(() => {
      if (this.turns.get(pubkey) === turn) this.sought.delete(pubkey);
      done();
    });
  }

  /**
   * A relay is connected again after none was: the profiles not found yet
   * (asked while offline, or the subscription lost with the connection)
   * are asked again, so the key becomes a name without leaving the screen.
   */
  reconnected() {
    for (const [pubkey, p] of Object.entries(this.profiles)) {
      if (p !== null) continue;
      this.sought.delete(pubkey);
      this.seek(pubkey);
    }
  }

  /** A profile arrived from the network: read it again. */
  refresh(pubkey: string) {
    if (!(pubkey in this.profiles) && !this.asked.has(pubkey)) return;
    this.asked.delete(pubkey);
    this.ask(pubkey);
  }
}

export const nameStore = new NameStore();
