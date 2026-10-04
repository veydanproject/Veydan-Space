// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

export type PassSection = 'passwords' | 'totp' | 'generator';

/**
 * The desktop UI state of the pass module. In Space the top bar's buttons and
 * the tray toggle the global drawers; in Pass, whose whole window is the Pass
 * page, there are no drawers (`drawers` stays false) and the same requests
 * show the page's pane instead (`section`).
 */
class PassUiStore {
  generatorOpen = $state(false);
  totpOpen = $state(false);
  passwordsOpen = $state(false);
  /** The global drawers are mounted (Space); otherwise the Pass page answers the requests. */
  drawers = $state(false);
  /** The pane the Pass page shows when it is one pane at a time (a narrow window). */
  section = $state<PassSection>('passwords');
  /** The generator's last password and form, shared by its pane and its drawer. */
  generated = $state('');
  generatedShown = $state(false);
  generatorSettingsOpen = $state(false);
}

export const passUi = new PassUiStore();
