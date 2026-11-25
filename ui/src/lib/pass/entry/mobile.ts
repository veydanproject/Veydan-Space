// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The pass module as the mobile shell sees it (platform-spec 11.1): three
// tiles of the Home grid — passwords, 2FA codes and the generator among the tools.
// A wide window shows a password or a code's form beside its list.

import { goto } from '$app/navigation';
import type { ModuleDef } from '$lib/core/module';
import { common } from './common';
import { withInline } from '$lib/pass/entities';
import PasswordRowInline from '$lib/pass/mobile/PasswordRowInline.svelte';
import PasswordRowDetails from '$lib/pass/mobile/PasswordRowDetails.svelte';
import TotpRowInline from '$lib/pass/mobile/TotpRowInline.svelte';
import MobilePasswordsPage from '$lib/pass/routes/MobilePasswordsPage.svelte';
import MobileTotpPage from '$lib/pass/routes/MobileTotpPage.svelte';

/** `/passwords/…` beside the passwords, `/totp/…` beside the codes; the scanner takes the whole screen. */
const lists = [
  { home: '/passwords', list: MobilePasswordsPage },
  { home: '/totp', list: MobileTotpPage },
];

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
  // The scanner takes the whole screen; the other screens use the root bar.
  bar: (url) => (url.pathname.startsWith('/totp/scan') ? null : undefined),
  split: (url) => {
    const path = url.pathname;
    if (path.startsWith('/totp/scan')) return undefined;
    return lists.find((l) => path === l.home || path.startsWith(l.home + '/'));
  },
};
