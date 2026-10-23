// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the module Communities is regardless of the shell
// (internal/communities-stage1.md §6.3): the channels, a section of the
// messenger's list of chats («Каналы»). It requires the messenger
// (products.json): the registry keeps it off while the messenger is off.
// In Space it has a switch of its own, in the Modules section of the
// settings; in Chat it follows the messenger and has none. It has no tab
// and no page of its own: its rows and its pages are the messenger's
// section, given at its start and taken back at its stop, so a module
// switched off leaves no menu entry, no row and no badge.

import type { ModuleDef } from '$lib/core/module';
import { startChannels, stopChannels } from '../section';

export const common: Omit<ModuleDef, 'nav' | 'settings'> = {
  id: 'communities',
  title: 'chan_module',
  icon: 'speakerphone',
  routes: [],
  requires: ['messenger'],
  start: startChannels,
  stop: stopChannels,
};
