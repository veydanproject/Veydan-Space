// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The sheet of the product's apps (HubSheet): one state for the shell, which
// shows the sheet, and for the button that opens it (the logo in the header of a tab, core/ui/HubButton).

class Hub {
  /** The product has apps to choose between; a product of one module has none. */
  available = $state(false);
  isOpen = $state(false);
  open(): void {
    if (this.available) this.isOpen = true;
  }
  close(): void {
    this.isOpen = false;
  }
}

export const hub = new Hub();
