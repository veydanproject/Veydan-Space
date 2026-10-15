// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { moderationError } from './errors';

const tr = (key: string) => key;

describe('a refusal of the moderation of a group', () => {
  it('names the part this build of the core does not do yet, as a code or in the text', () => {
    expect(moderationError({ code: 'not_implemented', message: 'not_implemented' }, tr)).toBe('msg_err_not_implemented');
    expect(moderationError('not_implemented: group_mod', tr)).toBe('msg_err_not_implemented');
  });

  it('is otherwise a refusal of the group module, or the text as it came', () => {
    expect(moderationError({ code: 'group_not_permitted', message: 'group_not_permitted' }, tr)).toBe('msg_err_group_not_permitted');
    expect(moderationError({ code: 'group_muted_until', message: 'group_muted_until' }, tr)).toBe('msg_err_group_muted_until');
    expect(moderationError(new Error('the relay is away'), tr)).toBe('the relay is away');
  });
});
