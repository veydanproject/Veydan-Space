// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1
// Copied from ui/src/lib/messenger/groups/groupStore.svelte.ts at 8ce09c39, then changed for channels.

// Channels as the module's runtime sees them: the rows of the list of
// chats (`channels_list`), the page of each one opened (`channel_get`),
// the invitations for me, what waits for a moderator, and my files on
// their way. The posts and comments are read by the pages themselves
// (`channel_history`, `channel_thread`) into the feed store
// (feedStore.svelte.ts); a `channel.message` bumps the revision of a
// channel, which those pages follow, and tells the feed store that an item
// of mine on its way is there.

import {
  channelErrorCode,
  channelsApi,
  errorCode,
  errorText,
  listenChannels,
  type ChannelAction,
  type ChannelDetail,
  type ChannelUiEvent,
  type ChannelView,
  type InviteView,
  type LevelInput,
  type LevelView,
  type NewChannelInput,
  type NewScopeInput,
  type ScopeSetInput,
} from './api';
import { clearAsked, markAsked, type Asked } from './levels/request';
import { feedStore, type OutboxEvent } from './feedStore.svelte';
import type { TransferProgress } from '$lib/messenger/seams/shared';
import { typingStore } from '$lib/messenger/seams/typing';

/** The module or its channels are not open yet (the app is starting, no identity): `channel.ready` comes when they are. */
export function notReady(e: unknown): boolean {
  return channelErrorCode(e) === 'channel_no_session' || errorCode(e) === 'communities_stopped' || /^communities_stopped\b/.test(errorText(e));
}

/** A file of mine on its way, as `channel.media` tells of it. */
export interface UploadProgress {
  id: string;
  communityId: string;
  /** The file's name; empty until the row of the runtime is read. */
  name: string;
  /** A recording made in the app (`voice`, `circle`): the line names it by its kind. */
  kind?: string;
  /** The post a comment goes under; empty for a post (unknown until the row is read). */
  postId?: string;
  status: string;
  done: number;
  total: number;
  error?: string;
  /** How it goes, as a chat's transfer tells it: stage, speed, time left (absent until it moves). */
  progress?: TransferProgress | null;
}

/** The chat id of a channel in the list of chats. */
export const chatIdOf = (communityId: string) => `channel:${communityId}`;

/** The channel of a chat id; `null` for a chat of something else. */
export const communityOf = (chatId: string | null | undefined): string | null =>
  chatId?.startsWith('channel:') ? chatId.slice(8) || null : null;

class ChannelStore {
  /** The rows, in the order of the runtime. */
  list = $state<ChannelView[]>([]);
  /** The pages read, by community id. */
  details = $state<Record<string, ChannelDetail>>({});
  /** Invitations waiting for my answer. */
  invites = $state<InviteView[]>([]);
  /** What waits for a moderator, by channel (`channel.queue`). */
  queue = $state<Record<string, number>>({});
  /** Requests to be let in, by channel, for those who answer them. */
  requests = $state<Record<string, number>>({});
  /** Requests for a level, by channel, for its moderators (communities-wire §14.4). */
  levelRequests = $state<Record<string, number>>({});
  /** Bumped by every `channel.message`: a page of a channel reads its feed again. */
  revision = $state<Record<string, number>>({});
  /**
   * Bumped by news of the journal of a channel (`channel.updated`, a
   * decision of a moderator, a request): what a page reads of the journal
   * (the log, the banned, the requests) follows this, not every message.
   */
  journalRev = $state<Record<string, number>>({});
  /** Bumped by `channel.keeper`: the state of the keeper and the tickets are read again. */
  keeperRev = $state<Record<string, number>>({});
  /** My files on their way, by upload id. */
  uploads = $state<Record<string, UploadProgress>>({});
  /** The files of messages coming down now, by message id, as `channel.media` tells: gone once here. */
  fetches = $state<Record<string, TransferProgress>>({});
  /** The levels I asked for, by channel, until the channel tells of a change (communities-wire §14.4). */
  levelAsked = $state<Asked>({});
  /** The rows were read: until then the section says it is loading, not that there are none. */
  loaded = $state(false);

  private timers = new Map<string, ReturnType<typeof setTimeout>>();
  private listTimer: ReturnType<typeof setTimeout> | null = null;
  private stopListening: (() => void) | null = null;

  byChat(chatId: string | null | undefined): ChannelView | null {
    const id = communityOf(chatId);
    return id ? (this.list.find((c) => c.id === id) ?? null) : null;
  }

  detail(communityId: string | null | undefined): ChannelDetail | null {
    return communityId ? (this.details[communityId] ?? null) : null;
  }

  /** The unread of the channels not muted and not archived: the badge of the section. */
  get unread(): number {
    return this.list.reduce((n, c) => n + (c.muted || c.archived || c.membership !== 'joined' ? 0 : c.unread), 0);
  }

  private retry: ReturnType<typeof setTimeout> | null = null;

  /**
   * The rows and the invitations. While the channels are not open yet (the
   * module starts after the page) nothing is said of them: `channel.ready`
   * reads them again, and so does a try a little later in case that event
   * went before this page listened.
   */
  async load() {
    let early = false;
    const [list, invites] = await Promise.all([
      channelsApi.list().catch((e) => { early = notReady(e); return null; }),
      channelsApi.invites('in').catch(() => [] as InviteView[]),
    ]);
    if (this.retry) clearTimeout(this.retry);
    this.retry = null;
    if (early && !this.loaded) {
      this.retry = setTimeout(() => { this.retry = null; void this.load(); }, 2000);
      return;
    }
    if (list) this.list = list;
    this.invites = (invites ?? []).filter((i) => i.status === 'received');
    this.loaded = true;
  }

  async refreshList() {
    this.list = (await channelsApi.list()) ?? [];
  }

  /** The rows are read again, once for a burst. */
  scheduleList() {
    if (this.listTimer) return;
    this.listTimer = setTimeout(() => {
      this.listTimer = null;
      this.refreshList().catch(() => {});
    }, 250);
  }

  /**
   * The page of a channel as a command or a read gave it. The same page read
   * again changes nothing: what follows it (the counts, the keeper, the tabs
   * of the moderation) is not asked again for a page that did not change.
   */
  private put(d: ChannelDetail) {
    const was = this.details[d.channel.id];
    if (was && JSON.stringify(was) === JSON.stringify(d)) return was;
    this.details = { ...this.details, [d.channel.id]: d };
    const i = this.list.findIndex((c) => c.id === d.channel.id);
    this.list = i < 0 ? [d.channel, ...this.list] : this.list.map((c, k) => (k === i ? d.channel : c));
    return d;
  }

  private drop(communityId: string) {
    const { [communityId]: _gone, ...rest } = this.details;
    this.details = rest;
    this.list = this.list.filter((c) => c.id !== communityId);
    feedStore.dropChannel(communityId);
  }

  /**
   * The page of a channel read again. Only a channel this device does not
   * know (`channel_unknown`: forgotten here, or never) goes; any other
   * failure (the module busy or not open yet, the vault locked) keeps what
   * is shown and is thrown to whoever asked.
   */
  async refresh(communityId: string) {
    try {
      this.put(await channelsApi.get(communityId));
    } catch (e) {
      if (channelErrorCode(e) === 'channel_unknown') this.drop(communityId);
      else throw e;
    }
  }

  /** What waits for a moderator and the requests, for a page of a channel I moderate. */
  async refreshCounts(communityId: string) {
    const [queue, requests, levels] = await Promise.all([
      channelsApi.moderation.queue(communityId, undefined, 1).then((p) => p.total).catch(() => 0),
      channelsApi.moderation.requestsCount(communityId).catch(() => 0),
      channelsApi.levels.requests(communityId).then((r) => r?.length ?? 0).catch(() => 0),
    ]);
    this.queue = { ...this.queue, [communityId]: queue };
    this.requests = { ...this.requests, [communityId]: requests };
    this.levelRequests = { ...this.levelRequests, [communityId]: levels };
  }

  async refreshInvites() {
    this.invites = ((await channelsApi.invites('in')) ?? []).filter((i) => i.status === 'received');
  }

  /** A burst of operations becomes one read. */
  private schedule(communityId: string) {
    if (this.timers.has(communityId)) return;
    this.timers.set(communityId, setTimeout(() => {
      this.timers.delete(communityId);
      this.refresh(communityId).catch(() => {});
    }, 250));
  }

  /**
   * My files of a channel on their way, as the runtime keeps them: a file
   * stored is gone from the list (its post came as `channel.message`), a
   * failed or paused one stays with its reason until it is cancelled or
   * goes on. Its size is the row's: a photo made smaller is smaller.
   */
  async refreshUploads(communityId: string) {
    const rows = (await channelsApi.uploads(communityId)) ?? [];
    const kept = Object.fromEntries(Object.entries(this.uploads).filter(([, u]) => u.communityId !== communityId));
    for (const r of rows) {
      const was = this.uploads[r.id];
      kept[r.id] = {
        id: r.id, communityId, name: r.name, kind: r.kind, postId: r.post_id, status: r.status, error: r.error ?? was?.error,
        done: Math.max(r.done, was?.done ?? 0), total: r.total, progress: was?.progress ?? null,
      };
    }
    this.uploads = kept;
  }

  /** Stops a file of mine on its way, or takes a failed one away. */
  async cancelUpload(uploadId: string) {
    const u = this.uploads[uploadId];
    await channelsApi.mediaCancel(uploadId);
    const { [uploadId]: _gone, ...rest } = this.uploads;
    this.uploads = rest;
    if (u) this.refreshUploads(u.communityId).catch(() => {});
  }

  /** Pauses a file of mine on its way: what the hosts hold stays; its word comes as `channel.media`. */
  async pauseUpload(uploadId: string) {
    await channelsApi.mediaPause(uploadId);
  }

  /** "Retry now" of a file of mine waiting for its next attempt. */
  async wakeUpload(uploadId: string) {
    await channelsApi.mediaWake(uploadId);
  }

  /** A failed or paused file of mine sent again, on from what the hosts hold; its progress comes as `channel.media`. */
  async retryUpload(uploadId: string) {
    const r = await channelsApi.mediaRetry(uploadId);
    const was = this.uploads[r.id]?.progress;
    this.uploads = {
      ...this.uploads,
      [r.id]: {
        id: r.id, communityId: r.community_id, name: r.name, kind: r.kind, postId: r.post_id, status: r.status, done: r.done, total: r.total,
        progress: was ? { ...was, status: 'queued', failure_reason: null } : null,
      },
    };
  }

  private bump(communityId: string) {
    this.revision = { ...this.revision, [communityId]: (this.revision[communityId] ?? 0) + 1 };
  }

  private bumpJournal(communityId: string) {
    this.journalRev = { ...this.journalRev, [communityId]: (this.journalRev[communityId] ?? 0) + 1 };
  }

  async create(input: NewChannelInput) {
    return this.put(await channelsApi.create(input));
  }

  async openLink(link: string, note?: string) {
    return this.put(await channelsApi.openLink(link, note));
  }

  async act(communityId: string, action: ChannelAction) {
    return this.put(await channelsApi.act(communityId, action));
  }

  /** A picked file (a path, or what the picker of a phone gives) as the cover. */
  async setAvatar(communityId: string, src: string) {
    return this.put(await channelsApi.avatarSet(communityId, src));
  }

  async removeAvatar(communityId: string) {
    return this.put(await channelsApi.avatarRemove(communityId));
  }

  async rotateLink(communityId: string) {
    return this.put(await channelsApi.rotateLink(communityId));
  }

  async invite(communityId: string, who: string) {
    return channelsApi.invite(communityId, who);
  }

  async answerInvite(inviteId: string, accept: boolean) {
    await channelsApi.answerInvite(inviteId, accept);
    this.invites = this.invites.filter((i) => i.invite_id !== inviteId);
    if (accept) this.scheduleList();
  }

  async answerRequest(communityId: string, requester: string, approve: boolean) {
    this.put(await channelsApi.answerRequest(communityId, requester, approve));
    this.refreshCounts(communityId).catch(() => {});
  }

  // ── Levels and sections (communities-wire §14) ─────────────────────────

  /** The levels a command answered with, put into the page of the channel at once. */
  private putLevels(communityId: string, levels: LevelView[]) {
    const d = this.details[communityId];
    if (d) this.details = { ...this.details, [communityId]: { ...d, levels } as ChannelDetail };
  }

  async addScope(communityId: string, input: NewScopeInput) {
    return this.put(await channelsApi.levels.scopeAdd(communityId, input));
  }

  async delScope(communityId: string, scopeId: string) {
    return this.put(await channelsApi.levels.scopeDel(communityId, scopeId));
  }

  async setScope(communityId: string, scopeId: string, input: ScopeSetInput) {
    return this.put(await channelsApi.levels.scopeSet(communityId, scopeId, input));
  }

  async orderScopes(communityId: string, ids: string[]) {
    return this.put(await channelsApi.levels.scopesOrder(communityId, ids));
  }

  async setScopePhoto(communityId: string, scopeId: string, src: string) {
    return this.put(await channelsApi.levels.scopePhotoSet(communityId, scopeId, src));
  }

  async setLevel(communityId: string, level: LevelInput) {
    const levels = await channelsApi.levels.set(communityId, level);
    this.putLevels(communityId, levels);
    return levels;
  }

  async delLevel(communityId: string, id: number) {
    const levels = await channelsApi.levels.del(communityId, id);
    this.putLevels(communityId, levels);
    return levels;
  }

  /** I ask the moderators for a level; the request is shown as sent until the channel changes. */
  async askLevel(communityId: string, level: number, note?: string) {
    await channelsApi.levels.request(communityId, level, note);
    this.levelAsked = markAsked(this.levelAsked, communityId, level);
  }

  /** A moderator gives a level asked for (`set_levels`) or refuses it with a reason. */
  async answerLevelRequest(communityId: string, requester: string, level: number, grant: boolean, reason?: string) {
    await channelsApi.levels.requestAct(communityId, requester, level, grant, reason);
    this.levelRequests = { ...this.levelRequests, [communityId]: Math.max(0, (this.levelRequests[communityId] ?? 1) - 1) };
    if (grant) this.schedule(communityId);
    this.refreshCounts(communityId).catch(() => {});
  }

  /** A person's levels (the base one among them); the journal's answer is the page. */
  async setMemberLevels(communityId: string, who: string, levels: number[]) {
    return this.put(await channelsApi.levels.setFor(communityId, who, levels));
  }

  /** What of mine the outbox of a channel gave up on, tried again now; each row tells of itself as `channel.outbox`. */
  async retryOutbox(communityId: string) {
    return this.put(await channelsApi.outboxRetry(communityId));
  }

  async forget(communityId: string) {
    await channelsApi.forget(communityId);
    this.drop(communityId);
  }

  /**
   * Gives up waiting for the keeper's key: a channel joined without a key
   * is left first (the journal hears it), then the channel goes from this
   * device. A leave that fails (no network) still forgets it here.
   */
  async cancelJoin(communityId: string) {
    if (this.details[communityId]?.channel.membership === 'joined') await channelsApi.act(communityId, { op: 'leave' }).catch(() => {});
    await this.forget(communityId);
  }

  /** Runtime event → state. */
  handleEvent(ev: ChannelUiEvent) {
    const p = (ev.payload ?? {}) as Record<string, unknown>;
    const cid = typeof p.community_id === 'string' ? p.community_id : typeof p.channel === 'string' ? p.channel : null;
    switch (ev.name) {
      case 'channel.updated':
        // Levels given (or anything else changed): a request sent is no longer said as waiting.
        if (cid) this.levelAsked = clearAsked(this.levelAsked, cid);
        if (cid) this.schedule(cid);
        if (cid) this.bumpJournal(cid);
        this.scheduleList();
        break;
      case 'channel.keeper':
        if (cid) this.schedule(cid);
        if (cid) this.keeperRev = { ...this.keeperRev, [cid]: (this.keeperRev[cid] ?? 0) + 1 };
        this.scheduleList();
        break;
      case 'channel.message':
        if (typeof p.id === 'string') feedStore.heardOf(p.id);
        if (cid) {
          this.bump(cid);
          if (Object.values(this.uploads).some((u) => u.communityId === cid)) this.refreshUploads(cid).catch(() => {});
        }
        this.scheduleList();
        break;
      // Somebody writes a comment under a post: the messenger keeps who types, here as in its chats.
      case 'channel.typing':
        typingStore.handleEvent(ev);
        break;
      // The channels of the identity are open: the rows are read (the first time, or for another identity).
      case 'channel.ready':
        void this.load();
        break;
      case 'channel.invite':
        this.refreshInvites().catch(() => {});
        break;
      // A reader asks for a level: the count of the moderation is read again.
      case 'channel.level_request':
        if (cid) this.refreshCounts(cid).catch(() => {});
        break;
      case 'channel.request':
        if (cid) {
          this.refreshCounts(cid).catch(() => {});
          this.schedule(cid);
          this.bumpJournal(cid);
        }
        break;
      // The queue grew or shrank, a moderator decided: the counts and the
      // page (a mute, a ban) are read again; the event only signals.
      case 'channel.queue':
        if (cid && typeof p.pending === 'number') this.queue = { ...this.queue, [cid]: p.pending };
        if (cid) this.schedule(cid);
        if (cid) this.bumpJournal(cid);
        break;
      case 'channel.moderation':
        if (cid) {
          this.schedule(cid);
          this.bump(cid);
          this.bumpJournal(cid);
        }
        break;
      // Something of mine went to the relays, or did not: the item is marked
      // at once (feedStore), and the page reads its pending marks and the
      // outbox of the channel again.
      case 'channel.outbox':
        feedStore.outbox(p as OutboxEvent);
        if (cid) {
          this.schedule(cid);
          this.bump(cid);
        }
        break;
      case 'channel.media': {
        const upId = typeof p.id === 'string' ? p.id : null;
        if (!cid || !upId) break;
        const status = String(p.status ?? '');
        const progress = p.progress && typeof p.progress === 'object' ? (p.progress as TransferProgress) : null;
        // The file of a message coming down is no row of mine: its progress
        // is kept by the message until it is here.
        if (status === 'downloading' || progress?.direction === 'down') {
          if (progress && progress.status !== 'done' && progress.status !== 'cancelled') this.fetches = { ...this.fetches, [upId]: progress };
          else if (upId in this.fetches) {
            const { [upId]: _here, ...rest } = this.fetches;
            this.fetches = rest;
          }
          break;
        }
        if (status === 'uploading' || status === 'paused' || status === 'failed') {
          const was = this.uploads[upId];
          // The last word of a run (paused, failed) comes without progress: what was told stays, with that word.
          const told = progress ?? (was?.progress ? { ...was.progress, status: status === 'uploading' ? was.progress.status : (status as TransferProgress['status']) } : null);
          this.uploads = {
            ...this.uploads,
            [upId]: {
              id: upId, communityId: cid, status, name: was?.name ?? '', kind: was?.kind, postId: was?.postId,
              done: Number(p.done ?? was?.done ?? 0),
              total: Number(p.total ?? was?.total ?? 0),
              error: typeof p.error === 'string' ? p.error : undefined,
              progress: told,
            },
          };
        } else {
          const { [upId]: _done, ...rest } = this.uploads;
          this.uploads = rest;
          this.bump(cid);
        }
        break;
      }
    }
  }

  /** The module's runtime is listened to; the rows and the invitations are read. */
  async start() {
    if (!this.stopListening) {
      const off = await listenChannels((ev) => this.handleEvent(ev));
      if (this.stopListening) off();
      else this.stopListening = off;
    }
    await this.load();
  }

  stop() {
    this.stopListening?.();
    this.stopListening = null;
    this.reset();
  }

  reset() {
    for (const t of this.timers.values()) clearTimeout(t);
    this.timers.clear();
    if (this.retry) clearTimeout(this.retry);
    this.retry = null;
    feedStore.reset();
    this.list = [];
    this.details = {};
    this.invites = [];
    this.queue = {};
    this.requests = {};
    this.levelRequests = {};
    this.uploads = {};
    this.fetches = {};
    this.levelAsked = {};
    this.revision = {};
    this.journalRev = {};
    this.keeperRev = {};
    this.loaded = false;
  }
}

export const channelStore = new ChannelStore();
