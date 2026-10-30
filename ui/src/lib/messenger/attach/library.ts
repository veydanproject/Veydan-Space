// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The one place that says which source of media the app uses.

import { isTauriHost } from '../api';
import { androidMediaLibrary } from './AndroidMediaLibrary';
import { dialogLibrary } from './DialogLibrary';
import { mockLibrary } from './MockLibrary';
import type { MediaLibrary } from './MediaLibrary';
import { systemPickerLibrary } from './SystemPickerLibrary';

/**
 * The grid of the phone's gallery, with the permissions to photos and
 * videos. `false`: the system's Photo Picker and the camera alone, without
 * those permissions — the switch for the day Google Play refuses their
 * declaration (ROO-110 §7); the manifests then drop them too.
 */
export const PHONE_GRID = true;

const android = typeof navigator !== 'undefined' && /android/i.test(navigator.userAgent);

export function mediaLibrary(): MediaLibrary {
  if (!isTauriHost) return mockLibrary;
  if (android) return PHONE_GRID ? androidMediaLibrary : systemPickerLibrary;
  return dialogLibrary;
}
