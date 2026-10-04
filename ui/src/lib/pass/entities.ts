// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * The pass module as owner of the kinds `password` and `totp` in the catalog
 * of entities (platform-spec 10.1). Secrets never leave through here: the
 * actions copy to the clipboard, the fields are the open ones.
 */

import type { Component } from 'svelte';
import { api } from '$lib/pass/api';
import { directory } from '$lib/core/directory';
import type { EntityKindDef } from '$lib/core/module';
import { parseBinding } from '$lib/core/bindings';
import { userLabels } from '$lib/core/entity-tags';
import { totpStore } from '$lib/pass/store/totp.svelte';
import { passwordStore } from '$lib/pass/store/passwords.svelte';

/** Backend clipboard: actions run after awaited calls, where `navigator.clipboard` is denied. */
async function copyText(text: string): Promise<void> {
  await api.system.clipboardWriteText(text);
}

/** Desktop writes the clipboard in Rust. Mobile falls back to reveal. */
async function copyPassword(id: string): Promise<void> {
  try {
    await api.passwords.copy(id);
    return;
  } catch (e) {
    const code = e && typeof e === 'object' && 'code' in e ? String((e as { code: string }).code) : '';
    if (code === 'vault_locked' || code === 'vault_mismatch' || code === 'decrypt_failed' || code === 'not_found') {
      throw e;
    }
  }
  const revealed = await api.passwords.reveal(id, 'password');
  try {
    await navigator.clipboard.writeText(revealed.value);
  } catch {
    await copyText(revealed.value);
  }
  const value = revealed.value;
  setTimeout(() => {
    navigator.clipboard.readText().then((current) => {
      if (current === value) void navigator.clipboard.writeText('');
    }).catch(() => {});
  }, 30_000);
}

/** Workspace color, then a free label, then the accent. Shared by chips and context cards. */
export function tagsColor(tags: string[]): string {
  for (const tag of tags) {
    const parsed = parseBinding(tag);
    if (parsed?.kind !== 'workspace') continue;
    const color = directory.color('workspace', parsed.value);
    if (color) return color;
  }
  for (const name of userLabels(tags)) {
    const color = directory.summary('note_tag', name)?.color;
    if (color) return color;
  }
  return 'var(--accent)';
}

export function totpName(id: string): string {
  const entry = totpStore.list.find((e) => e.id === id);
  if (!entry) return '';
  return entry.issuer ? `${entry.issuer} · ${entry.name}` : entry.name;
}

/** The ids of the passwords and TOTP entries that carry the tag `key` (`note:<id>`). */
const tagged = (list: { id: string; tags: string[] }[], key: string) => list.filter((e) => e.tags.includes(key)).map((e) => e.id);

export const passEntities: EntityKindDef[] = [
  {
    kind: 'totp',
    icon: 'key',
    label: 'ctx_kind_totp',
    color: 'var(--text-2)',
    ensureLoaded: () => totpStore.ensureLoaded(),
    list: () =>
      totpStore.list.map((e) => ({
        id: e.id,
        name: e.issuer ? `${e.issuer} · ${e.name}` : e.name,
        subtitle: '',
        status: null,
        color: tagsColor(e.tags),
      })),
    // Used by the command palette; the context card shows the live code chip instead
    actions: [
      {
        id: 'copy-code',
        label: 'ctx_action_copy_code',
        icon: 'copy',
        run: async (id) => {
          const code = await api.totp.generateCode(id);
          await copyText(code.code);
        },
      },
    ],
    fields: (id) => {
      const e = totpStore.list.find((x) => x.id === id);
      return e ? { name: e.name, issuer: e.issuer ?? '' } : undefined;
    },
    referring: (key) => tagged(totpStore.list, key),
    open: (id) => { totpStore.pendingSearch = totpName(id); },
  },
  {
    kind: 'password',
    icon: 'lock',
    label: 'ctx_kind_password',
    color: 'var(--text-2)',
    ensureLoaded: () => passwordStore.ensureLoaded(),
    list: () =>
      passwordStore.list.map((e) => ({
        id: e.id,
        name: e.title,
        subtitle: e.username ?? e.url ?? '',
        status: null,
        color: tagsColor(e.tags),
      })),
    actions: [
      {
        id: 'copy-username',
        label: 'ctx_action_copy_username',
        icon: 'copy',
        run: async (id) => {
          const entry = passwordStore.list.find((e) => e.id === id);
          if (entry?.username) await copyText(entry.username);
        },
      },
      {
        id: 'copy-password',
        label: 'ctx_action_copy_password',
        icon: 'copy',
        run: copyPassword,
      },
    ],
    fields: (id) => {
      const e = passwordStore.list.find((x) => x.id === id);
      return e ? { title: e.title, username: e.username ?? '', url: e.url ?? '' } : undefined;
    },
    referring: (key) => tagged(passwordStore.list, key),
    // A password tagged `note:<id>` mirrors the tag as `password:<id>` on the note (both ways).
    link: {
      add: (id, reference) => {
        const p = parseBinding(reference);
        return p?.kind === 'note' ? passwordStore.linkNote(id, p.value) : Promise.resolve();
      },
      remove: (id, reference) => {
        const p = parseBinding(reference);
        return p?.kind === 'note' ? passwordStore.unlinkNote(id, p.value) : Promise.resolve();
      },
    },
    open: (id) => { passwordStore.openId = id; },
  },
];

/** The kinds with the components one shell shows inside other modules' lists. */
export function withInline(
  parts: Partial<Record<string, { inline?: Component<{ id: string }>; details?: Component<{ id: string }>; open?: (id: string) => void }>>,
): EntityKindDef[] {
  return passEntities.map((def) => ({ ...def, ...parts[def.kind] }));
}
