// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The web cabinet on this device (cloud-api.md §16.4): the live sessions
// of the cabinet and the freeze of the code sign-in, read when the block
// «Входы на сайт» is shown, and what this run did to the cards of the bot —
// a sign-in denied, a session ended — so a card that was acted on offers
// its button no more. Nothing is kept past the run: a code lives five
// minutes, a session a day, and the server answers a repeat with 200.

import { asCloudError, webApi, type CloudError, type CloudWebSession, type CloudWebState } from '$lib/cloud/api';

/** Refusals that say a sign-in is over: «Это не я» has nothing left to deny. */
const OVER_CODES = ['login_expired', 'login_denied', 'login_used', 'not_found'];

export class WebStore {
  /** The live sessions; null until read, or when the server did not answer. */
  sessions = $state<CloudWebSession[] | null>(null);
  /** The code sign-in is frozen until then; null: not frozen, or the server does not say. */
  frozenUntil = $state<number | null>(null);
  /** A command is under way: its buttons wait. */
  busy = $state(false);
  error = $state<CloudError | null>(null);
  /** Sign-ins denied in this run (or refused as over), by id: the card of the code offers no «Это не я». */
  denied = $state<Record<string, CloudWebState>>({});
  /** Sessions ended in this run, by id: the card of the sign-in offers no «Завершить сессию». */
  ended = $state<Record<string, true>>({});

  async load(): Promise<void> {
    try {
      const got = await webApi.sessions();
      this.sessions = got.sessions;
      this.frozenUntil = got.frozen_until;
      this.error = null;
    } catch (e) {
      this.sessions = null;
      this.error = asCloudError(e);
    }
  }

  /** «Это не я» of a card or of the screen of approval. */
  async deny(id: string): Promise<CloudWebState | null> {
    return this.run(async () => {
      try {
        const done = await webApi.deny(id);
        this.denied = { ...this.denied, [id]: done };
        if (done.frozen_until) this.frozenUntil = done.frozen_until;
        return done;
      } catch (e) {
        const error = asCloudError(e);
        if (OVER_CODES.includes(error.code)) this.denied = { ...this.denied, [id]: { state: 'expired', frozen_until: null } };
        throw e;
      }
    });
  }

  /** «Завершить» one session, or every one. */
  async revoke(which: { id: string } | { all: true }): Promise<boolean> {
    const done = await this.run(async () => {
      await webApi.revoke(which);
      const gone = 'id' in which ? [which.id] : (this.sessions ?? []).map((s) => s.id);
      this.ended = { ...this.ended, ...Object.fromEntries(gone.map((id) => [id, true as const])) };
      this.sessions = (this.sessions ?? []).filter((s) => !gone.includes(s.id));
      return true;
    });
    return done ?? false;
  }

  /** «Снять заморозку входа по коду». */
  async unfreeze(): Promise<boolean> {
    const done = await this.run(async () => {
      await webApi.unfreeze();
      this.frozenUntil = null;
      return true;
    });
    return done ?? false;
  }

  /** One command at a time; a failure is kept in `error`, the answer is null. */
  private async run<T>(work: () => Promise<T>): Promise<T | null> {
    this.busy = true;
    this.error = null;
    try {
      return await work();
    } catch (e) {
      this.error = asCloudError(e);
      return null;
    } finally {
      this.busy = false;
    }
  }
}

export const webStore = new WebStore();
