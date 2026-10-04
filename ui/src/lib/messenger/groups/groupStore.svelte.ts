// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// Groups as the runtime sees them: members, roles, requests, the link,
// invitations for me. Messages of a group live in the chat store like
// any other conversation.

import {
  messengerApi,
  type GroupAction,
  type MessengerGroup,
  type MessengerGroupInvite,
  type MessengerUiEvent,
} from '../api';

class GroupStore {
  groups = $state<Record<string, MessengerGroup>>({});
  /** Invitations waiting for my answer. */
  invites = $state<MessengerGroupInvite[]>([]);
  private _timers = new Map<string, ReturnType<typeof setTimeout>>();

  byChat(chatId: string | null | undefined): MessengerGroup | null {
    if (!chatId?.startsWith('group:')) return null;
    return this.groups[chatId.slice(6)] ?? null;
  }

  /** Requests to my groups that wait for a manager. */
  get pendingRequests(): number {
    return Object.values(this.groups).reduce((n, g) => n + (g.membership === 'joined' ? g.requests.length : 0), 0);
  }

  private put(g: MessengerGroup) {
    this.groups = { ...this.groups, [g.id]: g };
  }

  async load() {
    const [list, invites] = await Promise.all([
      messengerApi.groups.list().catch(() => [] as MessengerGroup[]),
      messengerApi.groups.invites('in').catch(() => [] as MessengerGroupInvite[]),
    ]);
    this.groups = Object.fromEntries((list ?? []).map((g) => [g.id, g]));
    this.invites = invites ?? [];
  }

  async refresh(groupId: string) {
    try {
      this.put(await messengerApi.groups.get(groupId));
    } catch {
      // Forgotten on this device.
      const { [groupId]: _gone, ...rest } = this.groups;
      this.groups = rest;
    }
  }

  async refreshInvites() {
    this.invites = (await messengerApi.groups.invites('in')) ?? [];
  }

  /** A burst of operations (history) becomes one read. */
  private schedule(groupId: string) {
    if (this._timers.has(groupId)) return;
    this._timers.set(groupId, setTimeout(() => {
      this._timers.delete(groupId);
      this.refresh(groupId).catch(() => {});
    }, 250));
  }

  async create(kind: 'public' | 'private', name: string, about: string, historyForNew: boolean) {
    const g = await messengerApi.groups.create(kind, name, about, historyForNew);
    this.put(g);
    return g;
  }

  async openLink(link: string, note?: string) {
    const g = await messengerApi.groups.openLink(link, note);
    this.put(g);
    return g;
  }

  async act(groupId: string, action: GroupAction) {
    this.put(await messengerApi.groups.act(groupId, action));
  }

  async rotateLink(groupId: string) {
    this.put(await messengerApi.groups.rotateLink(groupId));
  }

  async invite(groupId: string, who: string) {
    return messengerApi.groups.invite(groupId, who);
  }

  async answerInvite(inviteId: string, accept: boolean) {
    await messengerApi.groups.answerInvite(inviteId, accept);
    this.invites = this.invites.filter((i) => i.invite_id !== inviteId);
  }

  async answerRequest(groupId: string, requester: string, approve: boolean) {
    this.put(await messengerApi.groups.answerRequest(groupId, requester, approve));
  }

  async forget(groupId: string) {
    await messengerApi.groups.forget(groupId);
    const { [groupId]: _gone, ...rest } = this.groups;
    this.groups = rest;
  }

  /** Runtime event → state. Called by the module store. */
  handleEvent(ev: MessengerUiEvent) {
    const p = (ev.payload ?? {}) as Record<string, unknown>;
    switch (ev.name) {
      case 'group.updated':
      case 'group.request':
        if (typeof p.group_id === 'string') this.schedule(p.group_id);
        break;
      case 'group.invite':
        this.refreshInvites().catch(() => {});
        break;
    }
  }

  reset() {
    this.groups = {};
    this.invites = [];
  }
}

export const groupStore = new GroupStore();
