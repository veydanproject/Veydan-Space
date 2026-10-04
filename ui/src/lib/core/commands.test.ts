// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { groupedInOrder, type Command } from './commands';

const cmd = (id: string, group: string, entity = false): Command => ({ id, title: id, group, entity, run: () => {} });

describe('the palette before anything is typed', () => {
  it("shows a group's commands and its entities' actions under one heading", () => {
    const list = [
      cmd('new-password', 'Password'),
      cmd('new-totp', 'TOTP'),
      cmd('copy-a', 'Password', true),
      cmd('copy-b', 'Password', true),
      cmd('code-a', 'TOTP', true),
    ];
    expect(groupedInOrder(list).map((c) => c.id)).toEqual(['new-password', 'copy-a', 'copy-b', 'new-totp', 'code-a']);
  });
});
