// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Who is writing now (platform-spec 6.3: a module sees of the messenger its
// seams alone). The channels hand their `channel.typing` to the messenger's
// one store of it and end the typing of an author whose comment came; the
// line itself is the messenger's `TypingLine` (seams/shared.ts). Apart from
// shared.ts so the channels' store takes the state alone, not the pages.

export { typingStore, type TypingScope } from '../typing/typingStore.svelte';
