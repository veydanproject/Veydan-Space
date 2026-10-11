// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The account of the product as the screens outside its owner show it: the
// card at the top of the settings and the page «Account» of the module
// cloud name the person and lead to what the owner of the key keeps. The
// owner (the messenger) answers through its description
// (`ModuleDef.account`); a product without it, or with it switched off, has
// no account, and nothing here names a module.

import type { AccountProfile, AccountSource } from './module';
import { registry } from './registry';

export type { AccountLink, AccountProfile, AccountSource } from './module';

/** The answers of the owner of the key, where the product has one and it is on. */
export function accountSource(): AccountSource | null {
  return registry.account() ?? null;
}

/** The account of this device; null without one, and where the owner did not answer. */
export async function loadAccount(): Promise<AccountProfile | null> {
  try {
    return (await accountSource()?.profile()) ?? null;
  } catch {
    return null;
  }
}

/** An npub short enough for a line: its head and its tail. */
export function shortNpub(npub: string): string {
  return npub.length > 20 ? `${npub.slice(0, 10)}…${npub.slice(-6)}` : npub;
}

/** The name a card shows: the profile's, or the short npub while there is none. */
export function accountName(profile: AccountProfile): string {
  return profile.name.trim() || shortNpub(profile.npub);
}
