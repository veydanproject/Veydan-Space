// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { goto } from '$app/navigation';
import { page } from '$app/state';
import { passUi, type PassSection } from '$lib/pass/store/ui.svelte';

/**
 * Show a part of pass on the desktop (the tray's generator, the palette's new
 * password): its drawer where the drawers are (Space), else the pane of the
 * Pass page, which in Pass is the whole window.
 */
export function showPass(section: PassSection): void {
  if (passUi.drawers) {
    if (section === 'generator') passUi.generatorOpen = true;
    else if (section === 'totp') passUi.totpOpen = true;
    else passUi.passwordsOpen = true;
    return;
  }
  passUi.section = section;
  if (!page.url.pathname.startsWith('/passwords')) void goto('/passwords');
}
