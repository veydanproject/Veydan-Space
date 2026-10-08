// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// The call nodes as the settings and the card of a link see them: one
// state, read by the panel of the network settings and by the card of a
// `veydan://call-node/…` link in a chat.

import { messengerApi, messengerError, messengerErrorCode, type CallNodesView, type CallTrust } from '../api';

/** A link of a private node with an invitation in it: adding it spends the invitation. */
export function carriesInvitation(text: string): boolean {
  const t = text.trim();
  return /^veydan:\/\/call-node\//i.test(t) && /[?&]t=[^&\s]+/.test(t);
}

/** A reference `address:port#id`, as the node's own tools print it: it may come with a key. */
export function isReference(text: string): boolean {
  return /^[^\s#/]+:\d{1,5}#[0-9a-fA-F]{64}$/.test(text.trim());
}

/**
 * What may come with a key given by hand: a reference, or a link of a node
 * without an invitation (`veydan://call-node/<id>?a=…`, its key told apart).
 */
export function takesKey(text: string): boolean {
  const t = text.trim();
  return isReference(t) || (/^veydan:\/\/call-node\//i.test(t) && !carriesInvitation(t));
}

/**
 * The `address:port` a link names, for the question before its invitation
 * is spent: as written when it does not decode (the runtime then refuses
 * the link itself, `call_node_invalid`), '' when there is none.
 */
export function linkAddress(text: string): string {
  const raw = /[?&]a=([^&#\s]+)/.exec(text.trim())?.[1] ?? '';
  try {
    return decodeURIComponent(raw);
  } catch {
    return raw;
  }
}

/** The failure as the screens word it: a locked vault is a code of its own here. */
function failureOf(e: unknown): string {
  if (messengerErrorCode(e) === 'vault_locked') return 'call_node_locked';
  return messengerError(e) || 'error';
}

class CallNodesStore {
  view = $state<CallNodesView | null>(null);
  busy = $state(false);
  /** A probe of the nodes is under way. */
  probing = $state(false);
  /** A failure of the last action of the panel. */
  error = $state('');
  /** The list was probed once since the panel opened: a reopening does not ask the nodes again. */
  private probed = false;

  /** Runs an action; its failure is the answer ('' on success) and, unless `quiet`, the panel's `error`. */
  private async attempt(action: () => Promise<CallNodesView>, quiet = false): Promise<string> {
    if (!quiet) this.error = '';
    this.busy = true;
    try {
      this.view = await action();
      return '';
    } catch (e) {
      const failure = failureOf(e);
      if (!quiet) this.error = failure;
      return failure;
    } finally {
      this.busy = false;
    }
  }

  /** The list as the runtime has it; then, once a session, a probe of the nodes calls may use. */
  async load() {
    await this.attempt(() => messengerApi.callNodes.list(false));
    if (!this.probed && this.view?.nodes.some((n) => n.used)) {
      this.probed = true;
      void this.probe();
    }
  }

  /** Asks the nodes calls may use now: round trips, loads and states come back. */
  async probe() {
    if (this.probing) return;
    this.probing = true;
    try {
      this.view = await messengerApi.callNodes.list(true);
    } catch (e) {
      this.error = failureOf(e);
    } finally {
      this.probing = false;
    }
  }

  add = (text: string, key?: string) => this.attempt(() => messengerApi.callNodes.add(text.trim(), key));
  /** From the card of a link: its failure is the card's alone. */
  addFromCard = (link: string) => this.attempt(() => messengerApi.callNodes.add(link), true);
  remove = (id: string) => this.attempt(() => messengerApi.callNodes.remove(id));
  setTrust = (trust: CallTrust) => this.attempt(() => messengerApi.callNodes.setTrust(trust));
  refreshRegistry = () => this.attempt(() => messengerApi.callNodes.refresh());

  /** Whether the node `id` is among mine, as far as the list says. */
  isMine(id: string): boolean {
    return Boolean(this.view?.nodes.some((n) => n.id === id && n.mine));
  }

  /**
   * Whether this device holds its own credentials on the node `id` (an
   * invitation was exchanged): an entry of my own list, with a shared key or
   * none, still leaves an invitation to the node to be exchanged.
   */
  holdsCredentials(id: string): boolean {
    return Boolean(this.view?.nodes.some((n) => n.id === id && n.mine && n.source === 'device'));
  }

  clearError() {
    this.error = '';
  }

  reset() {
    this.view = null;
    this.error = '';
    this.probed = false;
  }
}

export const callNodesStore = new CallNodesStore();
