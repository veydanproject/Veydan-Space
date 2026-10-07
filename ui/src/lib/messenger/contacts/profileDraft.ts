// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * The profile editor's unpublished form, kept while the editor is not on
 * screen: leaving it by anything but Cancel or Publish (another chat, the
 * media settings, the phone's back arrow) keeps the typing, and Edit opens
 * it again as it was. In memory only, for this run of the app, and only
 * for the identity it was typed for.
 */

import type { MessengerProfileInput } from '../api';

export interface ProfileDraft {
  /** The identity's hex key the form was typed for. */
  owner: string;
  /** The profile the form started from: what "changed" is measured against. */
  start: MessengerProfileInput;
  form: MessengerProfileInput;
  about: string;
  phone: string;
  share: boolean;
  privStart: { phone: string; share: boolean } | null;
}

let kept: ProfileDraft | null = null;

export const profileDraft = {
  /** Whether a form waits for `owner`. */
  has(owner: string | null | undefined): boolean {
    return !!owner && kept?.owner === owner;
  },
  /** The form waiting for `owner`, taken out (another identity's is dropped). */
  take(owner: string | null | undefined): ProfileDraft | null {
    const d = owner && kept?.owner === owner ? kept : null;
    kept = null;
    return d;
  },
  keep(d: ProfileDraft) {
    kept = structuredClone(d);
  },
  clear() {
    kept = null;
  },
};
