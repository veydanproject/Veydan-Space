// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The cross of a .search-field (base.css) clears the field and leaves the focus in it.

/** Called from the cross's click after the field is cleared: the focus goes back to the field's
    input before the cross, shown only while there is text, leaves the page. */
export function refocusSearch(e: Event): void {
  (e.currentTarget as HTMLElement | null)?.closest('.search-field')?.querySelector('input')?.focus();
}
