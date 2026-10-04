// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/** The ssh module as owner of the kind `ssh` in the catalog of entities (platform-spec 10.1). */

import { goto } from '$app/navigation';
import { api } from '$lib/ssh/api';
import type { EntityKindDef } from '$lib/core/module';
import { sshStore } from '$lib/ssh/store/ssh.svelte';

async function copyText(text: string): Promise<void> {
  await api.system.clipboardWriteText(text);
}

export const sshEntities: EntityKindDef[] = [
  {
    kind: 'ssh',
    icon: 'terminal',
    label: 'ctx_kind_ssh',
    color: 'var(--text-2)',
    ensureLoaded: () => sshStore.ensureLoaded(),
    list: () =>
      sshStore.connections.map((c) => ({
        id: c.id,
        name: c.name,
        subtitle: `${c.username}@${c.host}${c.port !== 22 ? `:${c.port}` : ''}`,
        status: sshStore.activeSession(c.id) ? 'ok' : null,
        color: 'var(--text-2)',
      })),
    actions: [
      { id: 'connect', label: 'ctx_action_connect', icon: 'terminal', run: async (id) => { await sshStore.connect(id); } },
      {
        id: 'copy',
        label: 'ctx_action_copy_host',
        icon: 'copy',
        run: async (id) => {
          const c = sshStore.connections.find((x) => x.id === id);
          if (c) await copyText(c.host);
        },
      },
    ],
    open: () => { void goto('/terminal'); },
    // Template placeholders of a note: `ssh`, `ssh_host`, `ssh_port`, `ssh_user`.
    fields: (id) => {
      const c = sshStore.connections.find((x) => x.id === id);
      if (!c) return undefined;
      return { name: c.name, host: c.host, port: String(c.port), user: c.username };
    },
  },
];
