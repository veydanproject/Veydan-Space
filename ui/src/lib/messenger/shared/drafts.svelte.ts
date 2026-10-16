// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The unsent text of each conversation, while the app runs: the composer
// parks it when the conversation changes, the chat list shows «Черновик:».
// Kept in memory only (a draft is plain text, the database is not).

import { SvelteMap } from 'svelte/reactivity';

export const drafts: SvelteMap<string, string> = ((globalThis as Record<string, unknown>).__msgDrafts ??= new SvelteMap()) as SvelteMap<string, string>;
