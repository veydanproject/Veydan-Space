// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import { describe, expect, it } from 'vitest';
import { module as mobile } from '../entry/mobile';
import { chatSettingsSections } from './chatSettingsSections';
import { SETTINGS_TABS, settingsUi } from './settingsTab.svelte';

describe('the group «Чаты» of the settings', () => {
  it('is the six tabs on a computer, in one group after the account (order 1) and before the browser (10)', () => {
    const sections = chatSettingsSections();
    expect(sections.map((s) => s.props?.tab)).toEqual([...SETTINGS_TABS]);
    expect(new Set(sections.map((s) => s.group))).toEqual(new Set(['msg_nav_chats']));
    for (const s of sections) {
      expect(s.order).toBeGreaterThan(1);
      expect(s.order).toBeLessThan(10);
    }
  });

  it('opens a tab by the module\'s own state and follows it', () => {
    const [profile, network] = chatSettingsSections();
    network.onopen?.();
    expect(settingsUi.tab).toBe('network');
    expect(network.current?.()).toBe(true);
    expect(profile.current?.()).toBe(false);
  });

  it('is one titled card of rows after the account on a phone', () => {
    expect(mobile.settings.map((s) => [s.id, s.group, s.after])).toEqual([['chats', 'msg_nav_chats', 'account']]);
  });
});
