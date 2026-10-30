// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The phone without the access to photos (platform-spec 6.3): the system's
// Photo Picker, which needs no permission, and the system camera; no grid.
// The way the app goes if Google Play does not take the declaration of the
// permissions to photos and videos: `library.ts` switches to it, the sheet
// and its items stay as they are.

import { androidMediaLibrary } from './AndroidMediaLibrary';
import { noGrid, type MediaLibrary } from './MediaLibrary';

export const systemPickerLibrary: MediaLibrary = {
  ...noGrid,
  capabilities: { pickSystem: true, grid: false, capture: true, photoPicker: true },
  capture: androidMediaLibrary.capture,
  pickSystem: androidMediaLibrary.pickSystem,
};
