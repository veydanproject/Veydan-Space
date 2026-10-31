// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The journal of calls (the section «Звонки»): its pages, and the count of
// missed calls on the section until it is looked at. The runtime keeps the
// record (`msg_calls`) and the mark «looked at»; a call that ends (of two
// or of a group) reads the journal again.

import { messengerApi, messengerError, type CallLogEntry, type MessengerUiEvent } from '../api';

const PAGE = 50;

class CallLogStore {
  entries = $state<CallLogEntry[]>([]);
  loaded = $state(false);
  /** No older page is left. */
  complete = $state(false);
  /** Missed calls since the journal was last shown: the badge of the section. */
  missed = $state(0);
  /** The journal is on the screen now: what comes is seen at once. */
  shown = $state(false);

  private loading: Promise<void> | null = null;

  /** The journal could not be read (the list says so, with «Повторить»). */
  failed = $state(false);
  /** What the runtime said when it failed, shown small under the words. */
  reason = $state('');

  /** The newest page, again (an older one comes with `more`). A failure is kept in `failed`, never a list that waits forever. */
  load(): Promise<void> {
    this.loading ??= (async () => {
      try {
        const page = await messengerApi.callLog.list(undefined, PAGE);
        this.entries = page;
        this.complete = page.length < PAGE;
        this.failed = false;
        this.reason = '';
      } catch (e) {
        this.reason = messengerError(e);
        this.failed = true;
      } finally {
        this.loaded = true;
        this.loading = null;
      }
    })();
    return this.loading;
  }

  async more() {
    if (this.complete || !this.entries.length) return;
    const before = this.entries[this.entries.length - 1].started_at;
    const page = await messengerApi.callLog.list(before, PAGE);
    const known = new Set(this.entries.map((e) => e.call_id));
    this.entries = [...this.entries, ...page.filter((e) => !known.has(e.call_id))];
    this.complete = page.length < PAGE;
  }

  async refreshMissed() {
    if (this.shown) return this.seen();
    this.missed = await messengerApi.callLog.missed();
  }

  /** The section is on the screen (or leaves it): its missed calls are no longer new. */
  async show(on: boolean) {
    this.shown = on;
    if (on) await Promise.all([this.load(), this.seen()]);
  }

  private async seen() {
    this.missed = 0;
    await messengerApi.callLog.markSeen();
  }

  /** Out of the journal; the lines in the chats stay. */
  async hide(callIds: string[]) {
    const gone = new Set(callIds);
    const before = this.entries;
    this.entries = before.filter((e) => !gone.has(e.call_id));
    try {
      await messengerApi.callLog.hide(callIds);
    } catch (e) {
      this.entries = before;
      throw e;
    }
    await this.refreshMissed().catch(() => {});
  }

  async clear() {
    const before = this.entries;
    this.entries = [];
    try {
      await messengerApi.callLog.clear();
    } catch (e) {
      this.entries = before;
      throw e;
    }
    this.complete = true;
    this.missed = 0;
  }

  /** Runtime event → the journal: a call that ended is in it now, or my other device changed it. Called by the module store. */
  handleEvent(ev: MessengerUiEvent) {
    if (ev.name !== 'call.ended' && ev.name !== 'group_call.ended' && ev.name !== 'call_log.updated') return;
    if (this.loaded) this.load().catch(() => {});
    this.refreshMissed().catch(() => {});
  }

  reset() {
    this.failed = false;
    this.entries = [];
    this.loaded = false;
    this.complete = false;
    this.missed = 0;
    this.shown = false;
  }
}

export const callLog = new CallLogStore();
