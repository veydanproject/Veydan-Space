// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// What the Cloud module is regardless of the shell (internal/vcloud-plan.md
// 2.2): a service of the product that follows the messenger — no switch of
// its own, never the default module, not counted when a product is told to
// have one module. Its page (`/cloud`) is the page «Account» of the
// product: who the account is, its plan and room, sync, the ways back into
// it; it opens from the account at the top of the settings, in Space and
// in Chat alike, and has no tab of its own. It requires the messenger, so
// the registry keeps it off, its entries and its page with it, while the
// messenger is off.

import type { ModuleDef, SetupItem } from '$lib/core/module';
import { call } from '$lib/core/api';
import { setCloudSource, type CloudStatusView, type CloudUsageView } from '$lib/core/sync/cloud';
import { cloudApi } from '$lib/cloud/api';
import { dropBotCards, takeBotCards } from '$lib/cloud/botCards';
import { ensureAccount } from '$lib/cloud/ensure';
import RecoveryStep from '$lib/cloud/RecoveryStep.svelte';

// The sync settings of the core ask the module of the account and its room
// (vcloud-plan.md 4.5, 4.7), and the first start has it make the account
// before it turns sync on. The commands are named here, in the module: a
// product without it (Notes, Pass) carries no name of them.
setCloudSource({
  status: () => call<CloudStatusView>('cloud_status'),
  usage: () => call<CloudUsageView>('cloud_usage'),
  ensure: () => ensureAccount(),
});

/** The Recovery Key on the page «Account»: where it is issued. */
export const RECOVERY_HREF = '/cloud?section=recovery-key';

/**
 * The 12 words, left to set up after the first start (cloud-recovery.md
 * §6): a line of the card «Finish setting up», where the identity has an
 * account. What the device keeps answers, so listing it asks the server
 * nothing.
 */
const recoveryWords: SetupItem = {
  id: 'recovery-words',
  order: 30,
  title: 'cloud_setup_words',
  hint: 'cloud_setup_words_hint',
  icon: 'key',
  href: () => RECOVERY_HREF,
  step: 'recovery',
  visible: async () => (await cloudApi.limits()).plan !== null,
  done: async () => (await cloudApi.recoveryStatus()).issued,
};

export const common: Omit<ModuleDef, 'nav' | 'settings'> = {
  id: 'cloud',
  title: 'cloud_title',
  icon: 'cloud',
  routes: ['/cloud'],
  service: true,
  // The messenger signs and sends for the cloud (products.json `requires`).
  requires: ['messenger'],
  // The cards of the bot in its chat are worded and acted on by this module (the messenger's seam, cloud-recovery.md §10).
  start: takeBotCards,
  stop: dropBotCards,
  // The last step of the first start: the Recovery Key, issued in the wizard itself.
  onboarding: [{ id: 'recovery', title: 'ob_step_protect', order: 40, component: RecoveryStep }],
  setup: [recoveryWords],
};
