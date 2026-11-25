// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// A bug report to the project goes the way a report of a person goes: by
// the messenger, signed by the account's key, kept on the device and tried
// again while there is no connection. The dialog is the core's
// (`BugReportDialog`); here is what it sends through. What is said when a
// report that waited is delivered or given up is `bugEvent.ts`, apart, so
// that the store that hears the events does not import itself.

import type { BugSender } from '$lib/core/module';
import { messengerApi, type NetDiagnosis } from '../api';
import { messengerStore } from '../store.svelte';

export const bugSender: BugSender = {
  // Only with the account whole and the project's servers in use: nothing
  // goes to the project from the user's own servers.
  async available() {
    await messengerStore.ensureLoaded().catch(() => {});
    return !!messengerStore.status?.runtime && !!messengerStore.identity && !messengerStore.pendingBackup
      && messengerStore.serversMode === 'veydan';
  },
  diagnose: () => messengerApi.net.diagnose(),
  async send(report) {
    const net = report.net as NetDiagnosis | undefined;
    const result = await messengerApi.bug.send({ ...report, net });
    return result.status;
  },
};
