// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { messengerTranslations as dictionary } from '../i18n';
import { netErrorKey, netErrorText } from './errors';

describe('the failures of adding a bridge', () => {
  it('tells a link of another kind from a broken one', () => {
    expect(netErrorKey('invalid input: net_bridge_link_type')).toBe('msg_bridge_error_link_type');
    expect(netErrorKey('net_bridge_invalid')).toBe('msg_bridge_error_invalid');
    expect(netErrorKey('storage error: disk full')).toBeNull();
  });

  it('has the texts in both languages, naming the link a bridge has', () => {
    for (const lang of ['en', 'ru'] as const) {
      expect(dictionary[lang].msg_bridge_error_link_type, lang).toContain('veydan://vlink/');
      expect(dictionary[lang].msg_bridge_add_hint, lang).toContain('veydan://vlink/');
      expect(dictionary[lang].msg_bridge_add_hint, lang).not.toContain('veydan://bridge/');
    }
  });
});

describe('the text a screen shows for a failure', () => {
  const translate = (key: string) => `<${key}>`;

  it('translates the known codes and keeps any other wording', () => {
    expect(netErrorText('invalid input: net_bridge_link_type', translate)).toBe('<msg_bridge_error_link_type>');
    expect(netErrorText('invalid input: net_bridge_invalid', translate)).toBe('<msg_bridge_error_invalid>');
    expect(netErrorText('storage error: disk full', translate)).toBe('storage error: disk full');
  });

  it('is empty without a failure', () => {
    expect(netErrorText('', translate)).toBe('');
  });
});
