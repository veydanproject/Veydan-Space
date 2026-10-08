// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { messengerTranslations as dictionary } from '../i18n';
import { callNodeErrorKey, callNodeErrorText, netErrorKey, netErrorText } from './errors';

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

describe('the failures of adding a call node', () => {
  const translate = (key: string) => `<${key}>`;

  it('words every code of the runtime and keeps any other wording', () => {
    expect(callNodeErrorText('call_node_bad_invite', translate)).toBe('<msg_call_nodes_err_bad_invite>');
    expect(callNodeErrorText('invalid input: call_node_link_type', translate)).toBe('<msg_call_nodes_err_link_type>');
    expect(callNodeErrorText('call_node_unreachable: call node: connection refused', translate)).toBe('<msg_call_nodes_err_unreachable>');
    expect(callNodeErrorText('call_node_invalid', translate)).toBe('<msg_calls_node_invalid>');
    expect(callNodeErrorKey('call_node_unreachable_soon')).toBeNull();
    expect(callNodeErrorText('storage error: disk full', translate)).toBe('storage error: disk full');
    expect(callNodeErrorText('', translate)).toBe('');
  });

  it('has the texts in both languages, naming the link a call node has', () => {
    for (const lang of ['en', 'ru'] as const) {
      expect(dictionary[lang].msg_call_nodes_err_link_type, lang).toContain('veydan://call-node/');
      expect(dictionary[lang].msg_calls_node_invalid, lang).toContain('veydan://call-node/');
      expect(dictionary[lang].msg_call_nodes_confirm_text, lang).toContain('{addr}');
    }
  });
});
