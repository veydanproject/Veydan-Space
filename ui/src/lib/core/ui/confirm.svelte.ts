// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * The app's confirmation question, in place of the browser's `confirm()`:
 * WebKitGTK answers that one without showing anything, and an Android
 * WebView may show nothing either, so a destructive action either went
 * through unasked or never happened. `ask()` opens the shell's
 * `ConfirmHost` (a dialog on the desktop, a bottom sheet on the phone) and
 * resolves with the answer; Escape, the backdrop and a second question
 * answer "no".
 */

export interface ConfirmRequest {
  title: string;
  message?: string;
  /** Default: Delete for a danger question, OK otherwise. */
  confirmLabel?: string;
  cancelLabel?: string;
  /** Default `danger`: every question of the app guards a deletion or a reset. */
  variant?: 'danger' | 'primary';
}

interface Pending extends ConfirmRequest {
  resolve: (yes: boolean) => void;
}

class ConfirmState {
  current = $state<Pending | null>(null);

  answer(yes: boolean): void {
    const pending = this.current;
    this.current = null;
    pending?.resolve(yes);
  }
}

export const confirmState = new ConfirmState();

/** Ask the question; true when the user confirmed. */
export function ask(request: ConfirmRequest): Promise<boolean> {
  confirmState.answer(false);
  return new Promise((resolve) => {
    confirmState.current = { ...request, resolve };
  });
}
