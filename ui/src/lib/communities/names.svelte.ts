// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from ui/src/lib/messenger/groups/names.svelte.ts at 8ce09c39, then changed for channels.

// Who is behind a key: my own name for a contact first, then what they
// call themselves, then a short form of the key. A channel names people
// as the messenger does, so a person reads the same in a channel and in a
// chat: the messenger's names are taken through its seam, not kept twice.

export { nameStore, shortKey } from '$lib/messenger/seams/shared';
