// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The page of Veydan Cloud on both platforms. In a product without the module
// the Vite plugin swaps this file for a stub that redirects to `/`
// (platform-spec 11.2); the module requires the messenger, so the guard
// sends the page to `/` while the messenger is off too.
import { guardModule } from '$lib/core/registry';

export const load = async () => {
  await guardModule('cloud');
};
