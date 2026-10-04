// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The passwords' phone screens (an entry, a new one). In a product without the module the Vite plugin swaps this
// file for a stub that redirects to `/` (platform-spec 11.2).
import { guardModule } from '$lib/core/registry';

export const load = () => guardModule('pass');
