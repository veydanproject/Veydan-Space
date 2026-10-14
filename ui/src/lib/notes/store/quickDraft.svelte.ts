// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * What was typed in Quick Capture and not saved. The lock gate unmounts the
 * page while the app is locked; the draft lives here, in memory only (never
 * in storage), so the page comes back with it after the unlock.
 */
export const quickDraft = $state({ text: '', tags: '' });
