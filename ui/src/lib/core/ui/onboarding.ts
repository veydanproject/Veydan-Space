// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The UI's way to the wizard of the first start: a module that may take only `$lib/core/ui/*`
// from the core (the messenger) opens it again at its step with it too.
export { onboarding } from '../onboarding/store.svelte';
