// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The sections of the chat list: «Чаты» are the chats between two (the
// bots and the requests of strangers among them), «Группы» the groups. A
// search goes across both: whatever is typed finds a chat in either.

/** Which section a chat is listed in. */
export type ChatSection = 'dm' | 'group';

export const chatKind = (c: { kind: string }): ChatSection => (c.kind === 'group' ? 'group' : 'dm');

/** Whether the list of `section` shows the chat: of its section, or of either while `query` (trimmed) is typed. */
export const inSection = (c: { kind: string }, section: ChatSection, query: string): boolean => query !== '' || chatKind(c) === section;
