// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Phone routes of the messenger. One place, so a host that mounts the
// module elsewhere changes a single constant.

export const BASE = '/messenger';

export const chatHref = (chatId: string) => `${BASE}/chat?id=${encodeURIComponent(chatId)}`;
