// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The call nodes of the settings over the commands of the runtime (here the
// mocks of the preview, net/callNodesDemo.ts, which answer as the runtime
// does): the trust level, a link with an invitation, a refusal worded, the
// card of a link failing on its own.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import cardSource from '../content/cards/CallNodeCard.svelte?raw';
import panelSource from './CallNodesPanel.svelte?raw';
import { demoCallNodeMocks } from './callNodesDemo';

const mocks = demoCallNodeMocks();
const asked: string[] = [];
const callNodes = vi.hoisted(() => ({}) as Record<string, (...a: unknown[]) => Promise<unknown>>);
vi.mock('../api', () => ({
  messengerApi: { callNodes },
  messengerError: (e: unknown) => (e && typeof e === 'object' && 'message' in e ? String((e as { message: string }).message) : String(e)),
  messengerErrorCode: (e: unknown) => (e && typeof e === 'object' && 'code' in e ? String((e as { code: string }).code) : null),
}));
const run = (cmd: string, args?: Record<string, unknown>) => async () => {
  asked.push(cmd);
  return mocks[cmd](args);
};
Object.assign(callNodes, {
  list: (probe = false) => run('messenger_call_nodes_list', { probe })(),
  add: (link: string, key?: string) => run('messenger_call_nodes_add', { link, key })(),
  remove: (id: string) => run('messenger_call_nodes_remove', { id })(),
  setTrust: (trust: string) => run('messenger_call_nodes_set_trust', { trust })(),
  refresh: () => run('messenger_call_nodes_refresh')(),
});

const { callNodesStore, carriesInvitation, isReference, linkAddress, takesKey } = await import('./callNodesStore.svelte');

const ID = 'ab'.repeat(32);
const LINK = `veydan://call-node/${ID}?a=203.0.113.7%3A8443&t=inv-1`;

describe('the call nodes of the settings', () => {
  beforeEach(() => {
    callNodesStore.reset();
    asked.length = 0;
  });

  it('loads the list and asks the nodes once a session', async () => {
    await callNodesStore.load();
    await vi.waitFor(() => expect(callNodesStore.probing).toBe(false));
    expect(callNodesStore.view?.nodes.some((n) => n.class === 'project')).toBe(true);
    expect(asked.filter((c) => c === 'messenger_call_nodes_list')).toHaveLength(2);
    await callNodesStore.load();
    expect(asked.filter((c) => c === 'messenger_call_nodes_list')).toHaveLength(3);
  });

  it('cuts what calls may use by the trust level', async () => {
    await callNodesStore.setTrust('own_only');
    expect(callNodesStore.view?.trust).toBe('own_only');
    expect(callNodesStore.view?.nodes.every((n) => n.used === n.mine)).toBe(true);
    await callNodesStore.setTrust('project_and_own');
    expect(callNodesStore.view?.nodes.find((n) => n.class === 'volunteer')?.used).toBe(false);
    await callNodesStore.setTrust('any');
  });

  it('adds a node by its link, mine and first, and removes it', async () => {
    expect(await callNodesStore.add(` ${LINK} `)).toBe('');
    const mine = callNodesStore.view?.nodes[0];
    expect(mine && [mine.id, mine.mine, mine.source, mine.class]).toEqual([ID, true, 'device', 'own']);
    expect(callNodesStore.isMine(ID)).toBe(true);
    expect(await callNodesStore.remove(ID)).toBe('');
    expect(callNodesStore.isMine(ID)).toBe(false);
  });

  it('keeps the code of a refusal for the screen to word', async () => {
    expect(await callNodesStore.add(`veydan://vlink/${ID}?a=1.2.3.4%3A443`)).toBe('call_node_link_type');
    expect(callNodesStore.error).toBe('call_node_link_type');
    // The card's failure is its own: the panel's stays as it was.
    expect(await callNodesStore.addFromCard(LINK.replace('inv-1', 'used'))).toBe('call_node_bad_invite');
    expect(callNodesStore.error).toBe('call_node_link_type');
  });

  it('tells a link that spends an invitation and a reference that takes a key', () => {
    expect(carriesInvitation(LINK)).toBe(true);
    expect(carriesInvitation(LINK.replace('&t=inv-1', ''))).toBe(false);
    expect(isReference(`203.0.113.7:8443#${ID}`)).toBe(true);
    expect(isReference(LINK)).toBe(false);
  });

  it('takes a key with a reference or with a link that carries no invitation', () => {
    expect(takesKey(`203.0.113.7:8443#${ID}`)).toBe(true);
    expect(takesKey(` ${LINK.replace('&t=inv-1', '')} `)).toBe(true);
    expect(takesKey(LINK)).toBe(false);
    expect(takesKey('nonsense')).toBe(false);
  });

  it('names the address of a link for the question, a malformed one as written', () => {
    expect(linkAddress(LINK)).toBe('203.0.113.7:8443');
    expect(linkAddress(`veydan://call-node/${ID}?a=%E0%A4%A&t=x`)).toBe('%E0%A4%A');
    expect(linkAddress(`veydan://call-node/${ID}?t=x`)).toBe('');
  });

  it('offers an invitation to a node of my own list until one was exchanged', async () => {
    expect(await callNodesStore.add(`203.0.113.7:8443#${ID}`, 'k1')).toBe('');
    expect([callNodesStore.isMine(ID), callNodesStore.holdsCredentials(ID)]).toEqual([true, false]);
    expect(await callNodesStore.add(LINK)).toBe('');
    expect([callNodesStore.isMine(ID), callNodesStore.holdsCredentials(ID)]).toEqual([true, true]);
    expect(await callNodesStore.remove(ID)).toBe('');
  });
});

describe('the screens of the call nodes', () => {
  it('word every failure and ask before an invitation is spent', () => {
    for (const [name, source] of Object.entries({ card: cardSource, panel: panelSource })) {
      expect(source, name).toContain('callNodeErrorText(');
      expect(source, name).toContain('ask(');
      expect(source, name).not.toMatch(/\{\s*callNodesStore\.error\s*\}/);
    }
  });
});

describe('the fixes of the review of the call nodes', () => {
  it('give the key with a link without an invitation, and never decode an address that may throw', () => {
    expect(panelSource).toContain('takesKey(text)');
    expect(panelSource).toContain('keyed ? key : undefined');
    expect(panelSource).not.toContain('decodeURIComponent');
  });

  it('say when the registry is paused, with no button to ask it', () => {
    expect(panelSource).toMatch(/\{#if !v\.registry_paused\}[\s\S]*refreshRegistry\(\)/);
    expect(panelSource).toContain("$t('msg_call_nodes_registry_paused')");
  });

  it('let the card of an invitation offer it unless this device holds its own credentials', () => {
    expect(cardSource).toContain('view.has_token ? callNodesStore.holdsCredentials(view.id) : callNodesStore.isMine(view.id)');
  });
});
