// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from ui/src/lib/messenger/groups/errors.test.ts at 8ce09c39, then changed for channels.

import { describe, expect, it } from 'vitest';
import { channelError, channelRefusal } from './errors';

const tr = (key: string, params?: Record<string, string>) => (key === 'chan_err_other' ? `${key}(${params?.code})` : key);

describe('a refusal of the module of channels', () => {
  it('is worded by its code, which starts the message of the error', () => {
    expect(channelError({ code: 'other', message: 'channel_not_permitted: not permitted' }, tr)).toBe('chan_err_not_permitted');
    expect(channelError({ code: 'other', message: 'channel_invite_only: this channel takes invitations only' }, tr)).toBe('chan_err_invite_only');
    expect(channelError('channel_no_key: no key of the channel yet', tr)).toBe('chan_err_no_key');
  });

  it('of a code this build does not know is «something went wrong» with the code', () => {
    expect(channelRefusal({ code: 'other', message: 'channel_from_the_future: …' }, tr)).toBe('chan_err_other(channel_from_the_future)');
  });

  it('is not a refusal when the text names no code; the text is said as it came', () => {
    expect(channelRefusal(new Error('the relay is away'), tr)).toBeNull();
    expect(channelError(new Error('the relay is away'), tr)).toBe('the relay is away');
  });

  it('says the lock closed when the vault is locked', () => {
    expect(channelError({ code: 'vault_locked', message: 'locked' }, tr)).toBe('chan_err_locked');
  });
});

describe('the refusals of a cover and of a file are said in words', () => {
  const said = (key: string, params?: Record<string, string>) => `${key}|${params?.answer ?? ''}`;

  it('names the host that refused and what it answered', () => {
    const e = { code: 'other', message: 'channel_upload_refused: channel: upload refused: https://media.example: err.auth_failed: no_account' };
    expect(channelError(e, said)).toBe('chan_err_upload_refused|https://media.example: err.auth_failed: no_account');
  });

  it('has words for a picture that is none and one too large', () => {
    expect(channelError({ code: 'other', message: 'channel_cover_unsupported: channel: a cover is not a picture' }, said)).toMatch(/^chan_err_cover_unsupported\|/);
    expect(channelError({ code: 'other', message: 'channel_cover_too_large: the file is too large' }, said)).toMatch(/^chan_err_cover_too_large\|/);
  });

  it('says that the channel\'s own server takes files from its team alone', () => {
    const e = { code: 'other', message: 'channel_storage_needs_key: err.storage_needs_key: no host of the channel takes a file of mine' };
    expect(channelError(e, said)).toMatch(/^chan_err_storage_needs_key\|/);
  });
});
