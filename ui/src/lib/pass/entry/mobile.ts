// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The pass module as the mobile shell sees it (platform-spec 11.1): three
// tiles of the Home grid — passwords, 2FA codes and the generator among the tools.

import { goto } from '$app/navigation';
import type { ModuleDef } from '$lib/core/module';
import { common } from './common';
import { withInline } from '$lib/pass/entities';
import PasswordRowInline from '$lib/pass/mobile/PasswordRowInline.svelte';
import PasswordRowDetails from '$lib/pass/mobile/PasswordRowDetails.svelte';
import TotpRowInline from '$lib/pass/mobile/TotpRowInline.svelte';

export const module: ModuleDef = {
  ...common,
  entities: withInline({
    password: { inline: PasswordRowInline, details: PasswordRowDetails, open: (id) => { void goto(`/passwords/${id}`); } },
    totp: { inline: TotpRowInline },
  }),
  nav: [
    { id: 'passwords', title: 'app_passwords', icon: 'lock-password', href: '/passwords' },
    { id: 'totp', title: 'app_totp', icon: 'shield-lock', href: '/totp' },
    { id: 'tools', title: 'app_tools', icon: 'dices', href: '/tools' },
  ],
  settings: [],
  // The scanner and the forms take the whole screen, as the note editor (design 16): no bar beside
  // their «Отмена | Сохранить»; the lists and the card use the root bar.
  bar: (url) => (isForm(url) ? null : undefined),
};

/** A form of a password or a 2FA code (a password's card in `?edit`), or the scanner. */
function isForm(url: URL): boolean {
  const path = url.pathname.replace(/\/$/, '');
  if (path === '/passwords/add' || /^\/totp\/[^/]+$/.test(path)) return true;
  return /^\/passwords\/[^/]+$/.test(path) && url.searchParams.has('edit');
}
