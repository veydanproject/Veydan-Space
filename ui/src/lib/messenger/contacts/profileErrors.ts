// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A refusal about my profile, my avatar or my phone, as a code and as the
// words the editor shows; the code also says which field it is about.

import { get } from 'svelte/store';
import { t, type TranslationKey } from '$lib/core/i18n';
import { messengerError, profileErrorCode } from '../api';

/** Where the editor shows a refusal. */
export type ProfileField = 'avatar' | 'bio' | 'website' | 'socials' | 'phone' | 'form';

/** The code of a refusal: the runtime's (`bio_too_long`, `media_no_server`…) or `website_invalid`. */
export function profileCodeOf(e: unknown): string | null {
  const code = profileErrorCode(e);
  if (code) return code;
  const text = e instanceof Error ? e.message : typeof e === 'string' ? e : messengerError(e);
  return /\bwebsite_invalid\b|website must be a URL/.test(text) ? 'website_invalid' : null;
}

/** The field a code is about. */
export function fieldOf(code: string | null): ProfileField {
  if (!code) return 'form';
  if (code === 'bio_too_long') return 'bio';
  if (code === 'website_invalid') return 'website';
  if (code.startsWith('social_')) return 'socials';
  if (code.startsWith('phone_')) return 'phone';
  if (code.startsWith('avatar_') || code === 'media_no_server') return 'avatar';
  return 'form';
}

/** The words for a refusal: its translation when it has a code, the runtime's message otherwise. */
export function profileErrorText(e: unknown): string {
  const code = profileCodeOf(e);
  const tr = get(t);
  if (code === 'website_invalid') return tr('msg_profile_website_invalid');
  if (code) return tr(`msg_err_${code}` as TranslationKey);
  return e instanceof Error ? e.message : messengerError(e);
}
