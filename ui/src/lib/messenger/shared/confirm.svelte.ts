// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// In-app confirmation. `window.confirm` is not available in every
// webview (Android), and a native box does not match the app anyway.

export interface ConfirmRequest {
  text: string;
  confirmLabel: string;
  danger: boolean;
}

class ConfirmStore {
  current = $state<ConfirmRequest | null>(null);
  private resolve: ((ok: boolean) => void) | null = null;

  ask(text: string, confirmLabel: string, danger = false): Promise<boolean> {
    this.resolve?.(false);
    this.current = { text, confirmLabel, danger };
    return new Promise((res) => { this.resolve = res; });
  }

  answer(ok: boolean) {
    this.current = null;
    this.resolve?.(ok);
    this.resolve = null;
  }
}

export const confirmStore = new ConfirmStore();
