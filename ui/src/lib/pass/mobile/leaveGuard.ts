// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A phone's form of Pass (a password, a 2FA code) does not lose what was typed
// unasked (design 16): its arrow, «Отмена» and the system «Назад» ask
// «Отменить изменения?» first, the red «Отменить» under the title, «Отмена»
// under it; a form without changes is left at once. The question is the
// messenger profile editor's (common_discard_confirm). Call during the page's
// init: it watches every way out of the page (a link, a back of the history).

import { afterNavigate, beforeNavigate, goto } from '$app/navigation';
import { get } from 'svelte/store';
import { t } from '$lib/core/i18n';
import { ask } from '$lib/core/ui/confirm.svelte';
import { afterBacks } from '$lib/core/ui/sheetBack';

export interface LeaveGuard {
  /** The form's own way out (its arrow, «Отмена»): asks if there are changes; true to go on, unasked. */
  mayLeave(): Promise<boolean>;
  /** The next way out goes unasked (after a save). */
  pass(): void;
}

const question = () =>
  ask({ title: get(t)('common_discard_confirm'), confirmLabel: get(t)('common_discard'), variant: 'danger' });

export function leaveGuard(dirty: () => boolean): LeaveGuard {
  let through = false;

  beforeNavigate((nav) => {
    // Closing the app cannot wait for an answer.
    if (through || nav.type === 'leave' || !dirty()) return;
    nav.cancel();
    void question().then(async (yes) => {
      if (!yes) return;
      through = true;
      await afterBacks();
      if (nav.type === 'popstate') history.go(nav.delta ?? -1);
      else if (nav.to) void goto(nav.to.url);
    });
  });
  // A way out let through once: the next stay of the form asks again.
  afterNavigate(() => {
    through = false;
  });

  return {
    async mayLeave() {
      if (through || !dirty()) return true;
      if (!(await question())) return false;
      through = true;
      return true;
    },
    pass() {
      through = true;
    },
  };
}
