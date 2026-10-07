// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The texts of the failures of a transfer. The runtime answers with a stable
// code (`err.*`) inside its message; a code this UI has no text for gets the
// general one, never the name of its key. Anything else is shown as it came.

import { mediaErrorCode, messengerError } from '../api';
import { messengerTranslations } from '../i18n';

export type MediaErrorKey = Extract<keyof typeof messengerTranslations.en, `msg_media_err_${string}`>;

/** Key of the text for a code: `msg_media_err_<code>`, or the general one. */
export function mediaErrorKey(code: string): MediaErrorKey {
  const key = `msg_media_${code.replace('.', '_')}`;
  return (key in messengerTranslations.en ? key : 'msg_media_err_unknown') as MediaErrorKey;
}

/** The text a screen shows for a failure, '' for none. */
export function mediaErrorText(e: unknown, translate: (key: MediaErrorKey) => string): string {
  if (!e) return '';
  const code = mediaErrorCode(e);
  return code ? translate(mediaErrorKey(code)) : typeof e === 'string' ? e : messengerError(e);
}

/**
 * The name a refusal of a file too large to send carries
 * (`err.file_too_large: IMG_1.mp4`, said by the import of a `content://`
 * pick, whose path names nothing a person reads); `null` without one.
 */
export function tooLargeName(e: unknown): string | null {
  const m = /\berr\.file_too_large: ?(.*)$/s.exec(typeof e === 'string' ? e : messengerError(e));
  return m?.[1].trim() || null;
}
