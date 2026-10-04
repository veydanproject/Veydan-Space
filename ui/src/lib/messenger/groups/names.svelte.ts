// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Who is behind a key: my own name for a contact first, then what they
// call themselves, then a short form of the key. Profiles of people who
// are not contacts are read once and kept for the session.

import { contactLabel, messengerApi, profileLabel, type MessengerProfile } from '../api';
import { messengerStore } from '../store.svelte';

export function shortKey(pubkey: string): string {
  return pubkey.length > 12 ? `${pubkey.slice(0, 6)}…${pubkey.slice(-4)}` : pubkey;
}

class NameStore {
  private profiles = $state<Record<string, MessengerProfile | null>>({});
  private asked = new Set<string>();

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
        .then((p) => { this.profiles = { ...this.profiles, [pubkey]: p ?? null }; })
        .catch(() => { this.profiles = { ...this.profiles, [pubkey]: null }; });
    });
  }

  /** A profile arrived from the network: read it again. */
  refresh(pubkey: string) {
    if (!(pubkey in this.profiles) && !this.asked.has(pubkey)) return;
    this.asked.delete(pubkey);
    this.ask(pubkey);
  }
}

export const nameStore = new NameStore();
