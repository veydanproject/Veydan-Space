// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { mediaErrorKey, mediaErrorText, tooLargeName } from './errors';

// That every code of the backend has its text: scripts/tests/media-errors.tests.mjs.
describe('the failures of a transfer', () => {
  it('shows the text of a code, the general one for an unknown code, and other wording as it came', () => {
    const translate = (key: string) => `<${key}>`;
    expect(mediaErrorKey('err.timeout')).toBe('msg_media_err_timeout');
    expect(mediaErrorText('transport error: err.timeout: slow', translate)).toBe('<msg_media_err_timeout>');
    expect(mediaErrorText('err.from_a_later_version', translate)).toBe('<msg_media_err_unknown>');
    expect(mediaErrorText('disk full', translate)).toBe('disk full');
    expect(mediaErrorText('', translate)).toBe('');
  });

  it('reads the name a refusal of a file too large carries', () => {
    expect(tooLargeName('err.file_too_large: IMG_0042.mp4')).toBe('IMG_0042.mp4');
    expect(tooLargeName({ code: 'other', message: 'err.file_too_large: Vacances 2026.mov' })).toBe('Vacances 2026.mov');
    expect(tooLargeName('err.file_too_large')).toBeNull();
    expect(tooLargeName('err.network')).toBeNull();
  });
});
