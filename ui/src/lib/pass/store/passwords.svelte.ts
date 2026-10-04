// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { api } from '$lib/pass/api';
import { directory } from '$lib/core/directory';
import type { PasswordEntry } from '$lib/pass/types';

class PasswordStore {
  list = $state<PasswordEntry[]>([]);
  loading = $state(false);
  loaded = $state(false);
  /** Set by the command palette; the password drawer opens the create form. */
  createRequest = $state(false);
  /** Password to show when the drawer opens. */
  openId = $state<string | null>(null);
  private _promise: Promise<void> | null = null;

  async ensureLoaded() {
    if (this.loaded) return;
    if (this._promise) return this._promise;
    this._promise = this.refresh().finally(() => { this._promise = null; });
    return this._promise;
  }

  async refresh() {
    this.loading = true;
    try {
      this.list = await api.passwords.list();
      this.loaded = true;
    } finally {
      this.loading = false;
    }
  }

  byProfile(profileId: string): PasswordEntry[] {
    const tag = `profile:${profileId}`;
    return this.list.filter((e) => e.tags.includes(tag));
  }

  countForProfile(profileId: string): number {
    return this.byProfile(profileId).length;
  }

  byWorkspace(workspaceId: string, profileIds: string[]): PasswordEntry[] {
    const wsTag = `workspace:${workspaceId}`;
    const profileTags = new Set(profileIds.map((id) => `profile:${id}`));
    return this.list.filter(
      (e) => e.tags.includes(wsTag) || e.tags.some((t) => profileTags.has(t)),
    );
  }

  /** Move a linked TOTP to the front; the first one shows in the password list. */
  async makePrimaryTotp(passwordId: string, totpId: string) {
    const entry = this.list.find((e) => e.id === passwordId);
    if (!entry || entry.totp_ids[0] === totpId || !entry.totp_ids.includes(totpId)) return;
    const next = [totpId, ...entry.totp_ids.filter((id) => id !== totpId)];
    await api.passwords.update(passwordId, { totp_ids: next });
    await this.refresh();
  }

  /** Remove the soft note tag and the leftover password binding on that note. */
  async unlinkNote(passwordId: string, noteId: string) {
    const tag = `note:${noteId}`;
    const entry = this.list.find((e) => e.id === passwordId);
    if (entry?.tags.includes(tag)) {
      await api.passwords.update(passwordId, { tags: entry.tags.filter((t) => t !== tag) });
      await this.refresh();
    }
    // The notes keep the reference on their side; the owner leaves a note without it alone.
    await directory.get('note')?.link?.remove(noteId, `password:${passwordId}`);
  }

  /** Mirror `note:{id}` tags onto notes as `password:{id}`; notes no longer tagged lose it. */
  async syncNoteBindings(passwordId: string, tags: string[]) {
    const notes = directory.get('note');
    if (!notes?.link || !notes.referring) return;
    await notes.ensureLoaded();
    const linked = new Set(tags.filter((t) => t.startsWith('note:')).map((t) => t.slice(5)));
    const binding = `password:${passwordId}`;
    const referring = notes.referring(binding);
    const stale = referring.filter((id) => !linked.has(id));
    const missing = [...linked].filter((id) => !referring.includes(id));
    // One note at a time: each call rewrites the shared notes manifest.
    for (const id of stale) await notes.link.remove(id, binding);
    for (const id of missing) await notes.link.add(id, binding);
  }

  /** Link from the note side: tag the password first so the note binding is not seen as stale. */
  async linkNote(passwordId: string, noteId: string) {
    await this.ensureLoaded();
    const entry = this.list.find((e) => e.id === passwordId);
    if (!entry) return;
    const tag = `note:${noteId}`;
    if (!entry.tags.includes(tag)) {
      await api.passwords.update(passwordId, { tags: [...entry.tags, tag] });
      await this.refresh();
    }
    await directory.get('note')?.link?.add(noteId, `password:${passwordId}`);
  }
}

export const passwordStore = new PasswordStore();
