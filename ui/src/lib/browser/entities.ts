// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

/**
 * The browser module as owner of the kinds `workspace`, `profile` and
 * `proxy` in the catalog of entities (platform-spec 10.1): how to load, name,
 * describe and act on each, for note chips, context cards, pickers and the
 * command palette of any module.
 */

import { get } from 'svelte/store';
import { goto } from '$app/navigation';
import { api } from '$lib/browser/api';
import { t } from '$lib/core/i18n';
import { isTor, torCountries, torLabel } from '$lib/browser/proxy-label';
import type { EntityKindDef } from '$lib/core/module';
import { workspacesStore } from '$lib/browser/store/workspaces.svelte';
import { profilesStore } from '$lib/browser/store/profiles.svelte';
import { proxiesStore } from '$lib/browser/store/proxies.svelte';
import { proxyEditor } from '$lib/browser/store/proxy-editor.svelte';

/** Backend clipboard: actions run after awaited calls, where `navigator.clipboard` is denied. */
async function copyText(text: string): Promise<void> {
  await api.system.clipboardWriteText(text);
}

const region = (country: string | null, city: string | null) =>
  [country, city].filter((s): s is string => !!s).join(', ');

export const browserEntities: EntityKindDef[] = [
  {
    kind: 'workspace',
    icon: 'layers',
    label: 'ctx_kind_workspace',
    color: 'var(--success)',
    ensureLoaded: () => workspacesStore.ensureLoaded(),
    list: () =>
      workspacesStore.list.map((w) => ({
        id: w.id,
        name: w.name,
        subtitle: w.description ?? '',
        status: null,
        color: w.color,
      })),
    actions: [
      { id: 'open', label: 'ctx_action_open', icon: 'external-link', run: async (id) => { await goto(`/workspace/${id}`); } },
    ],
    open: (id) => { void goto(`/workspace/${id}`); },
  },
  {
    kind: 'profile',
    icon: 'globe',
    label: 'ctx_kind_profile',
    color: 'var(--accent)',
    ensureLoaded: () => profilesStore.ensureLoaded(),
    list: () =>
      profilesStore.list.map((p) => ({
        id: p.id,
        name: p.name,
        subtitle: p.browser_type,
        status: p.status === 'running' ? 'ok' : 'unknown',
        color: 'var(--accent)',
      })),
    actions: [
      {
        id: 'launch',
        label: 'ctx_action_launch',
        icon: 'play',
        run: async (id) => {
          await api.profiles.launch(id);
          await profilesStore.refresh();
        },
      },
    ],
    open: (id) => {
      const ws = profilesStore.list.find((p) => p.id === id)?.workspace_id;
      if (ws) void goto(`/workspace/${ws}`);
    },
  },
  {
    kind: 'proxy',
    icon: 'shield',
    label: 'ctx_kind_proxy',
    color: 'var(--warn-text)',
    ensureLoaded: () => proxiesStore.ensureLoaded(),
    list: () =>
      proxiesStore.list.map((p) => ({
        id: p.id,
        name: p.name,
        subtitle: isTor(p)
          ? torLabel(p.country, get(t))
          : [p.proxy_type.toUpperCase(), region(p.country, p.city)].filter(Boolean).join(' · '),
        status: p.status === 'active' ? 'ok' : p.status === 'failed' ? 'bad' : 'unknown',
        color: 'var(--warn-text)',
      })),
    actions: [
      {
        id: 'check',
        label: 'ctx_action_check',
        icon: 'zap',
        run: async (id) => {
          try {
            await proxiesStore.check(id);
          } catch (e) {
            proxiesStore.markFailed(id);
            throw e;
          }
        },
      },
      {
        id: 'copy',
        label: 'ctx_action_copy',
        icon: 'copy',
        run: async (id) => {
          // The kind has one list of actions, so it cannot be hidden for one row: a Tor row has no URL.
          if (proxiesStore.list.find((p) => p.id === id)?.proxy_type === 'tor') throw new Error(get(t)('proxy_tor_copy_none'));
          await copyText(await api.proxies.exportUrl(id));
        },
      },
    ],
    open: () => { void goto('/proxies'); },
    edit: (id) => proxyEditor.open(id),
    // Template placeholders of a note: `proxy`, `proxy_type`, `proxy_host`, …
    fields: (id) => {
      const p = proxiesStore.list.find((x) => x.id === id);
      if (!p) return undefined;
      // A Tor row names no server: its host and port are a placeholder, and its country is the set of exit countries.
      if (isTor(p)) return { name: p.name, type: p.proxy_type, host: '', port: '', region: torCountries(p.country) };
      return { name: p.name, type: p.proxy_type, host: p.host, port: String(p.port), region: region(p.country, p.city) };
    },
  },
];
