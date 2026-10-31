<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->
<!-- Copied from ui/src/lib/messenger/groups/GroupChat.svelte at 8ce09c39, then changed for channels. -->

<!--
  A channel opened from the list of chats (communities-spec §12, «лента
  канала»): its posts top-down, the newest at the bottom as in a chat, the
  pinned post over them, reactions, and «Комментарии N» that opens the
  thread of a post beside the feed (over it on a phone). Authors post from
  the composer at the bottom, readers comment under the posts. Where I
  stand in the channel is said in place of the feed: waiting for the
  keeper's key, an old key catching up, a request sent, out of it.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { tick, untrack } from 'svelte';
  import { goto } from '$app/navigation';
  import { countKey, locale, localeTag, t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { copied, toast } from '$lib/core/ui/toast.svelte';
  import ContextMenu, { type MenuEntry } from '$lib/core/ui/ContextMenu.svelte';
  import {
    AttachButton, Avatar, Composer, MediaViewer, WithInfo, chatPrefs, confirmStore, dayLabel, listStamp, mediaTitle, myKey, onPhone, reportStore, viewer, writeTo,
  } from '$lib/messenger/seams/shared';
  import ChannelInfo from './ChannelInfo.svelte';
  import ThreadView from './ThreadView.svelte';
  import FeedPost from './FeedPost.svelte';
  import PickedTray from './PickedTray.svelte';
  import PostEditor from './PostEditor.svelte';
  import { composerOfDraft, postDrafts, splitPost, type PostDraft } from './postEditor';
  import { Picked, type AddOpts } from './picked.svelte';
  import { feedUnits } from './albums';
  import { channelStore, communityOf } from './channelStore.svelte';
  import UploadLine from './UploadLine.svelte';
  import { nameStore } from './names.svelte';
  import { offerToMe } from './transfer';
  import { channelsApi, isRecording, mediaOf, type ChannelRecording, type FeedItemView, type TextFormat } from './api';
  import { channelError } from './errors';
  import { itemEntries } from './itemMenu';
  import { channelEntries } from './channelMenu';
  import { feedKey, feedStore, isLocal, localId, scopeOfKey } from './feedStore.svelte';
  import { EVERYONE, hasSections, inScope, levelsOf, postTarget, scopeOf, sectionOf, sectionTiles, sectionTitle, type SectionTile } from './levels/sections';
  import { lookOf, ordered } from './levels/look';
  import SectionBadge from './levels/SectionBadge.svelte';
  import SectionTiles from './levels/SectionTiles.svelte';
  import ScopeDialog from './levels/ScopeDialog.svelte';
  import type { ScopeView } from './levels/types';
  import { levelFor } from './levels/request';
  import LevelRequestDialog from './levels/LevelRequestDialog.svelte';
  import { cover } from './cover';
  import { isMine, QUICK } from './reactions';
  import {
    amMuted, answersRequests, canAttach, catchUpNote, canComment, canPost, canSetUpSections, isModerator, isOwner, joined, keeperAwayAt, standing, staffOf, type Standing,
  } from './permissions';
  import type { ModerationTab } from './moderation/ModerationPanel.svelte';
  import PersonActions from './moderation/PersonActions.svelte';
  import { opensAt } from './moderation/schedule';
  import { outKey } from './lines';

  interface Props {
    /** `channel:<id>`. */
    chatId: string;
    onback?: () => void;
  }
  let { chatId, onback }: Props = $props();

  const phone = onPhone();
  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);
  const PAGE = 30;
  /** How long the events of a page the relays answered may take to reach the feed. */
  const SETTLE_MS = 2500;

  const cid = $derived(communityOf(chatId) ?? '');
  const detail = $derived(channelStore.detail(cid));
  const row = $derived(detail?.channel ?? channelStore.byChat(chatId));
  const me = $derived(myKey());
  const where = $derived<Standing | null>(detail ? standing(detail) : null);

  /** The panel beside the feed: the channel's info (with its moderation) or the thread of a post. */
  let panel = $state<'info' | 'thread' | null>(null);
  let moderation = $state(false);
  let modTab = $state<ModerationTab>('queue');
  let threadOf = $state<FeedItemView | null>(null);

  /** The section shown (its tile picked): its id, or `null` for «Все», all I am in. */
  let scope = $state<string | null>(null);
  /** A locked tile no level opens: why its section is closed, said under the head. */
  let locked = $state<Extract<SectionTile, { kind: 'locked' }> | null>(null);
  /** The tiles of the sections, open under the button of the head (a sheet on a phone). */
  let tilesOpen = $state(false);
  let sectionButton = $state<HTMLElement | null>(null);
  /** The settings of a section (the owner's and the admins'), from «Настроить» of the tiles. */
  let scopeDialog = $state<{ open: boolean; id: string | null }>({ open: false, id: null });

  /** The feed as read on this device (feedStore): a channel opened again shows it at once and reads its newest behind it. */
  const key = $derived(feedKey(cid, scope));
  const list = $derived(feedStore.list(key));
  const items = $derived(feedStore.shown(key));
  /** The posts drawn: a removed one leaves the feed, no «Удалено» in its place (the owner, 2026-10-24); `items` keeps it for the thread open on it. */
  const live = $derived(items.filter((x) => !x.deleted));
  const nextBefore = $derived(list?.next);
  /** The relays answered an empty page a moment ago: what they sent may still be on its way to the feed. */
  let settling = $state(false);
  let settleTimer: ReturnType<typeof setTimeout> | undefined;
  let loadingEarlier = $state(false);
  let error = $state('');
  let missing = $state(false);
  let highlighted = $state<string | null>(null);

  let scroller = $state<HTMLDivElement | null>(null);
  let atBottom = $state(true);
  /** Where the list on the screen is, from the bottom (`null`: at it), kept for the store when it is left. */
  let anchor: { key: string; from: number | null } = { key: '', from: null };

  // ── Reading ─────────────────────────────────────────────────────────────

  let shownFor = '';
  $effect(() => {
    const id = cid;
    if (id === shownFor) return;
    shownFor = id;
    untrack(() => {
      atBottom = true; settling = false; clearTimeout(settleTimer); settleTimer = undefined; error = ''; missing = false; panel = null; moderation = false; threadOf = null;
      scope = null; locked = null; tilesOpen = false; tray.clear(); editor = null;
      viewer.close();
      // The page of the channel kept here stays shown while it is read again; only a channel this device does not know goes.
      channelStore.refresh(id).then(() => { if (id === cid && !channelStore.detail(id)) missing = true; })
        .catch((e) => { if (id === cid && !channelStore.detail(id)) error = channelError(e, tr); });
      // Files of mine left on their way (failed, or broken off by a closed app) show with a way to send them again.
      channelStore.refreshUploads(id).catch(() => {});
    });
  });

  /**
   * The lists drawn (a switch of sections, 2026-10-23): the one shown and
   * those of the sections shown before it, hidden. A switch back shows its
   * cards as they were, nothing is drawn anew: drawing a feed's cards again
   * at every switch froze the page. In the order first drawn (a list is
   * never moved), the least recently shown going first past `KEEP_DRAWN`.
   */
  let drawn = $state<string[]>([]);
  let shownOrder: string[] = [];
  const KEEP_DRAWN = 6;
  /** A list drawn for the first time: how many of its newest posts are drawn; more each frame, until all are. */
  let partial = $state<Record<string, number>>({});
  const STEP_UNITS = 8;

  /** Where the list shown stands: at the newest, or as far from the bottom as it was left. */
  function place(k: string) {
    if (!scroller || k !== key) return;
    const from = anchor.key === k ? anchor.from : null;
    scroller.scrollTop = from === null ? scroller.scrollHeight : Math.max(0, scroller.scrollHeight - from);
  }

  // The list on the screen: kept from trimming while shown, left where it was read to; opened again there, or at the newest.
  $effect(() => {
    const k = key;
    const id = cid;
    untrack(() => {
      feedStore.enter(k);
      const from = feedStore.list(k)?.anchor ?? null;
      anchor = { key: k, from };
      atBottom = from === null;
      shownOrder = [...shownOrder.filter((x) => x !== k && feedStore.list(x) && x.startsWith(`${id}|`)), k];
      const keep = shownOrder.slice(-KEEP_DRAWN);
      shownOrder = keep;
      if (!drawn.includes(k)) partial = { ...partial, [k]: STEP_UNITS };
      drawn = [...drawn.filter((x) => keep.includes(x)), ...(drawn.includes(k) ? [] : [k])];
      void tick().then(() => {
        // A list hidden keeps its cards, and would keep playing what plays in them: it stops, as it did when they went.
        for (const m of scroller?.querySelectorAll<HTMLMediaElement>('.list:not(.on) video, .list:not(.on) audio') ?? []) m.pause();
        place(k);
      });
    });
    return () => feedStore.leave(k, anchor.key === k ? anchor.from : null);
  });

  // A list drawn for the first time, once read: its newest posts are shown first, then a few more each frame.
  $effect(() => {
    const k = key;
    const n = partial[k];
    const total = units.length;
    if (n === undefined || !list?.loaded) return;
    let frame = requestAnimationFrame(() => {
      frame = requestAnimationFrame(() => {
        if (n + STEP_UNITS >= total) {
          const { [k]: _p, ...rest } = partial;
          partial = rest;
        } else partial = { ...partial, [k]: n + STEP_UNITS };
        void tick().then(() => place(k));
      });
    });
    return () => cancelAnimationFrame(frame);
  });

  /** The posts of a list as drawn, of the section its key shows; the albums as one; a removed post is not drawn (as `live`). */
  const unitsOf = (k: string) => feedUnits(inScope(feedStore.shown(k).filter((x) => !x.deleted), scopeOfKey(k)));

  // The counts of what waits for a moderator, for the pills of the head: once a channel, not at every news of it.
  const moderating = $derived(isModerator(detail));
  $effect(() => {
    const id = cid;
    if (moderating) untrack(() => channelStore.refreshCounts(id).catch(() => {}));
  });

  /** «Loading» only until the first page of this list was read: a page read again never shows it over the feed. */
  const loading = $derived(!list?.loaded && !error);

  $effect(() => {
    const asked = !!list?.asked;
    untrack(() => settle(asked));
  });

  async function load() {
    const id = cid;
    const sc = scope;
    const k = key;
    if (!id) return;
    try {
      // As a chat: the newest page at the bottom, what was read above it stays; a burst of messages is one more read.
      await feedStore.refresh(k, () => channelsApi.history(id, undefined, PAGE, sc));
      if (k === key) error = '';
    } catch (e) {
      if (k === key) error = channelError(e, tr);
    }
    if (k === key && atBottom) await scrollDown();
  }

  /**
   * The empty state only once the relays had their say: their page came back
   * (`asked`) and what it brought is taken a moment later, as `channel.message`,
   * which reads the feed again. Said once per channel, so a feed read again
   * never goes back to «loading».
   */
  function settle(empty: boolean) {
    if (!empty) {
      clearTimeout(settleTimer);
      settling = false;
      return;
    }
    if (settleTimer) return;
    settling = true;
    settleTimer = setTimeout(() => (settling = false), SETTLE_MS);
  }

  async function loadEarlier() {
    if (!nextBefore || loadingEarlier) return;
    const id = cid;
    const sc = scope;
    const k = key;
    loadingEarlier = true;
    const el = scroller;
    const from = el ? el.scrollHeight - el.scrollTop : 0;
    try {
      const more = await feedStore.earlier(k, (next) => channelsApi.history(id, next, PAGE, sc));
      if (!more || k !== key) return;
      await tick();
      if (el) el.scrollTop = el.scrollHeight - from;
    } catch (e) {
      error = channelError(e, tr);
    } finally {
      loadingEarlier = false;
    }
  }

  // The feed is read when the channel can be read, and again at every
  // `channel.message` of it (a new post, an edit, a reaction, a decision);
  // not at every refresh of the channel's detail, which comes with any
  // news of the channel and would read the feed over and over.
  const readable = $derived(!!detail && (detail.channel.membership === 'joined' || detail.channel.membership === 'out') && detail.has_key);
  $effect(() => {
    void channelStore.revision[cid];
    void key;
    if (readable) untrack(() => load());
  });

  // What is shown is read: the unread of the row goes, here and on my other devices; the same newest is said once.
  $effect(() => {
    void items.length;
    const unread = row?.unread ?? 0;
    if (!joined(detail) || !atBottom || unread === 0) return;
    untrack(() => {
      const id = cid;
      const newest = items.filter((x) => !isLocal(x.id)).at(-1)?.created_at;
      if (newest !== undefined && (feedStore.readUpTo[id] ?? -1) >= newest) return;
      if (newest !== undefined) feedStore.readUpTo[id] = newest;
      channelsApi.read(id, newest).then(() => channelStore.scheduleList()).catch(() => {});
    });
  });

  async function scrollDown() {
    await tick();
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  }

  // A panel opened or closed beside the feed changes its width: the newest stays in sight.
  $effect(() => {
    void panel;
    untrack(() => { if (atBottom) void scrollDown(); });
  });

  function onscroll() {
    const el = scroller;
    if (!el) return;
    atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
    anchor = { key, from: atBottom ? null : el.scrollHeight - el.scrollTop };
    // Not while the newest alone are drawn: the top of the list is not its top yet.
    if (el.scrollTop < 120 && nextBefore && !loadingEarlier && partial[key] === undefined) void loadEarlier();
  }

  // ── Sections ────────────────────────────────────────────────────────────

  const lv = $derived(levelsOf(detail));
  /** «Все» and the sections; none while the channel has only everyone's. */
  const tiles = $derived(sectionTiles(lv.scopes, lv.levels));
  const sectioned = $derived(tiles.length > 0);
  /** What the section shown shows: the pages are asked of it, and what was read is sorted again here. */
  const shownItems = $derived(inScope(live, scope));
  /** What the list shown says: still read, nothing in its section, nothing at all, or its posts. */
  const feedState = $derived<'loading' | 'scope_empty' | 'empty' | 'feed'>(
    loading || (settling && live.length === 0) ? 'loading'
      : shownItems.length === 0 && scope !== null && !nextBefore ? 'scope_empty'
      : live.length === 0 && where !== 'out' ? 'empty'
      : 'feed',
  );
  /** The feed as drawn: the posts of one album (files sent together) as one. */
  const units = $derived(feedUnits(shownItems));
  /** The parts of each album by its last post, the one its menu, reactions and comments are of. */
  const albumOf = $derived(new Map(units.filter((u) => u.items.length > 1).map((u) => [u.items[u.items.length - 1].id, u.items])));
  const scopeLabel = (s: ScopeView) => (s.scope_id === EVERYONE && !s.title ? $t('chan_scope_everyone') : s.title);
  /** The section shown, its look and its name in the head; «Все» is the accent and the grid. */
  const shownScope = $derived(scope ? (lv.scopes.find((s) => s.scope_id === scope) ?? null) : null);
  const shownColor = $derived((shownScope ? lookOf(shownScope).color : null) ?? 'var(--accent)');
  const shownName = $derived(shownScope ? scopeLabel(shownScope) : $t('chan_sections_all'));
  /** Where a post goes (the owner's word, 2026-10-23): into the section shown, from «Все» into everyone's; `null`: I may not post there. */
  const target = $derived(postTarget(lv.scopes, scope));
  const targetScope = $derived(sectioned && target ? (lv.scopes.find((s) => s.scope_id === target) ?? null) : null);

  /** Another section: its list, as read before (at once) and from its newest again. */
  function pick(next: string | null) {
    locked = null;
    tilesOpen = false;
    if (next === scope) return;
    scope = next;
  }

  // The tiles open: the sections not read yet are read meanwhile, so the one picked shows its posts at once, not «loading».
  $effect(() => {
    if (!tilesOpen || !readable) return;
    const id = cid;
    const open = tiles.flatMap((t) => (t.kind === 'open' ? [t.scope.scope_id] : []));
    untrack(() => {
      for (const sc of open) {
        const k = feedKey(id, sc);
        if (!feedStore.list(k)) feedStore.refresh(k, () => channelsApi.history(id, undefined, PAGE, sc)).catch(() => {});
      }
    });
  });

  /** A locked tile: the request of the level that opens it; none opens it, the strip under the head says why. */
  function onLocked(tile: Extract<SectionTile, { kind: 'locked' }>) {
    const level = levelFor(tile.scope.scope_id, lv.levels, lv.my_levels, lv.scopes);
    if (level) { locked = null; asking = { open: true, level: level.id }; } else locked = tile;
  }

  /** «Настроить» (the owner and the admins, 2026-10-25): the section shown is set up, from «Все» the first. */
  function setUpSections() {
    scopeDialog = { open: true, id: scope ?? ordered(lv.scopes)[0]?.scope_id ?? EVERYONE };
  }

  // A section gone (deleted, or my level taken) leaves its tile: back to all.
  $effect(() => {
    const sc = scope;
    if (sc !== null && !lv.scopes.some((s) => s.scope_id === sc && (s.mine || s.scope_id === EVERYONE))) untrack(() => pick(null));
  });

  let asking = $state<{ open: boolean; level: number | null }>({ open: false, level: null });

  const lockedText = $derived.by(() => {
    if (!locked) return '';
    const title = locked.scope.title;
    return locked.levels.length
      ? $t('chan_scope_locked_text', { title, levels: locked.levels.map((l) => `«${l.name}»`).join(', ') })
      : $t('chan_scope_locked_none', { title });
  });

  /** A day of the feed starts a new date line. */
  const dayOf = (at: number) => new Date(at * 1000).toDateString();

  // ── Acting ──────────────────────────────────────────────────────────────

  async function run(fn: () => Promise<unknown>): Promise<boolean> {
    error = '';
    try { await fn(); return true; }
    catch (e) { error = channelError(e, tr); return false; }
  }

  /**
   * A post of mine stands in the feed at once, marked as on its way, and
   * gets its id when the command answers; a refusal takes it away and puts
   * the words back into the composer. `fmt`: the composer's Markdown menu was used.
   */
  async function send(text: string, fmt: TextFormat | null = null) {
    const body = text.trim();
    if (tray.files.length) return sendFiles(body, fmt);
    if (!body) return;
    const id = cid;
    const k = key;
    const sc = sectioned ? target : null;
    const temp = localId();
    error = '';
    feedStore.addLocal(k, {
      id: temp, community_id: id, post_id: '', author: me ?? '', created_at: Math.floor(Date.now() / 1000), text: body, fmt: fmt ?? undefined,
      review: 'ok', hidden: false, deleted: false, pinned: false, reactions: [], comments: 0, scope_id: sc ?? undefined, pending: true,
    });
    atBottom = true;
    void scrollDown();
    try {
      feedStore.settle(k, temp, await channelsApi.post(id, body, fmt, sc));
    } catch (e) {
      feedStore.dropLocal(k, temp);
      error = channelError(e, tr);
      throw new Error(error);
    }
    void load();
  }

  /** What of mine the outbox of the channel gave up on, sent again (all of it: the command retries the channel). */
  async function retryOutbox() {
    await run(() => channelStore.retryOutbox(cid));
  }

  /** Picked files wait above the composer for the words that go with them, as in a chat. */
  const tray = new Picked();
  function attach(paths: string[], opts?: AddOpts) {
    tray.add(paths, opts);
  }

  /**
   * What waits in the tray goes: one file as a post, several as one album,
   * the words as the caption of the last; each shows its progress at the
   * foot of the feed until it is a post.
   */
  async function sendFiles(caption: string, fmt: TextFormat | null = null) {
    const id = cid;
    const sc = sectioned ? target : null;
    error = '';
    atBottom = true;
    try {
      await tray.send((path, opts) => channelsApi.mediaSend(id, path, { scopeId: sc, ...opts }), caption, fmt);
    } catch (e) {
      error = channelError(e, tr);
      throw new Error(error);
    } finally {
      await channelStore.refreshUploads(id).catch(() => {});
      void scrollDown();
    }
  }

  /** My files on their way: a comment's shows under its post when its page is open beside the feed, here otherwise. */
  /** What the bar of the pinned post says of a file without words: its name, a voice message or a circle by its kind. */
  function pinnedMedia(p: FeedItemView): string {
    const m = mediaOf(p);
    return m ? (isRecording(m) ? mediaTitle(m.kind, m.name, $t) : m.name) : '';
  }

  const uploads = $derived(Object.values(channelStore.uploads).filter((u) => u.communityId === cid && !(u.postId && panel === 'thread' && threadOf?.id === u.postId)));

  /**
   * A voice message or a circle recorded in the composer goes as a post of
   * the section shown, as a picked file does: its line at the foot of the
   * feed until it is a post (2026-10-23).
   */
  async function record(rec: ChannelRecording) {
    const id = cid;
    const sc = sectioned ? target : null;
    error = '';
    atBottom = true;
    try {
      await channelsApi.mediaRecord(id, rec, { scopeId: sc });
    } catch (e) {
      error = channelError(e, tr);
      throw new Error(error);
    } finally {
      await channelStore.refreshUploads(id).catch(() => {});
      void scrollDown();
    }
  }

  /** Toggled on the tap; a refusal takes it back and says why. */
  async function react(item: FeedItemView, emoji: string) {
    const id = cid;
    await run(() => feedStore.react(id, item, emoji, (set) => channelsApi.react(id, item.id, emoji, set)));
  }

  // ── The extended editor (stage 6 of the redesign) ─────────────────────

  /** The composer of the feed: ⤢ takes its words to the editor, a close gives them back. */
  let composer = $state<ReturnType<typeof Composer> | null>(null);
  /** The editor over the pane: a new post (the composer's words and tray), or an edit of `item`. */
  let editor = $state<{ draft: PostDraft; item: FeedItemView | null } | null>(null);

  function expand(text: string, md: boolean) {
    editor = { draft: postDrafts.open(chatId, text, md), item: null };
  }

  /** «Изменить» of a post or of the caption of its files: the same editor, its title the first line. */
  function openEdit(it: FeedItemView) {
    editor = { draft: splitPost(it.text, it.fmt), item: it };
  }

  function closeEditor(d: PostDraft) {
    if (editor && !editor.item) {
      const back = composerOfDraft(d);
      composer?.restore(back.text, back.md);
    }
    editor = null;
  }

  /** The editor's post goes as the composer's does (with the tray's files, as their caption); an edit says how its words are written. */
  async function publishEdited(text: string, fmt: TextFormat | null) {
    const it = editor?.item;
    if (it) {
      if (!(await run(() => channelsApi.edit(cid, it.id, text, fmt)))) throw new Error(error);
    } else {
      await send(text, fmt);
    }
    editor = null;
  }

  /** Where the editor's post goes: the section picked (an edit: the post's own), named only when the channel has sections. */
  const editorSection = $derived.by(() => {
    if (!editor || !hasSections(detail)) return null;
    const id = editor.item ? scopeOf(editor.item) : target;
    const s = lv.scopes.find((x) => x.scope_id === id);
    return s ? { name: scopeLabel(s), color: lookOf(s).color, icon: lookOf(s).icon } : null;
  });

  /** A post removed; an album goes whole (its menu is the menu of its last post). */
  async function remove(item: FeedItemView) {
    const mineIt = item.author === me;
    if (!(await confirmStore.ask(tr(mineIt ? 'chan_delete_confirm' : 'chan_delete_other_confirm', { name: nameStore.label(item.author) }), tr('chan_menu_delete'), true))) return;
    const id = cid;
    const parts = albumOf.get(item.id) ?? [item];
    await run(async () => { for (const p of parts) await channelsApi.remove(id, p.id); });
  }

  function report(item: FeedItemView) {
    // The report names the channel and the post: the moderators read it on the channel's relays.
    reportStore.open({ accused: item.author, name: nameStore.label(item.author), context: { t: 'channel', community_id: cid, post_id: item.id }, item: item.post_id ? 'comment' : 'post', canBlock: false });
  }

  /** The channel itself: its owner answers for it, the context names it. */
  function reportChannel() {
    if (row) reportStore.open({ target: 'channel', accused: row.owner, name: row.name, context: { t: 'channel', community_id: cid }, canBlock: false });
  }

  function openThread(item: FeedItemView) {
    threadOf = item; panel = 'thread'; moderation = false;
  }

  function openInfo(tab?: ModerationTab) {
    if (tab) { modTab = tab; moderation = true; } else moderation = false;
    panel = 'info';
  }

  // ── The menu of a post ─────────────────────────────────────────────────

  let menu = $state<{ open: boolean; x: number; y: number; item: FeedItemView | null }>({ open: false, x: 0, y: 0, item: null });
  /** A moderator's acts on the author of a post, asked as from the team. */
  let acts = $state<PersonActions | null>(null);

  const entries = $derived.by((): MenuEntry[] => {
    const it = menu.item;
    if (!it || !detail) return [];
    return itemEntries(it, detail, me, tr, {
      comment: () => openThread(it),
      copy: () => { navigator.clipboard.writeText(it.text ?? '').then(() => copied()).catch(() => {}); },
      pin: (pinned) => { void run(() => channelsApi.pin(cid, it.id, pinned).then(() => channelStore.refresh(cid))); },
      edit: () => openEdit(it),
      remove: () => { void remove(it); },
      hide: () => { void run(() => channelsApi.moderation.mod(cid, { t: 'hide', targets: [it.id] })); },
      approve: () => { void run(() => channelsApi.moderation.mod(cid, { t: 'approve', target: it.id, version: it.id })); },
      reject: () => { void run(() => channelsApi.moderation.mod(cid, { t: 'reject', target: it.id })); },
      write: () => { writeTo(it.author).catch((e) => toast.error(channelError(e, tr))); },
      report: () => report(it),
      hideAuthor: () => acts?.askHide(it.author),
      mute: () => acts?.askMute(it.author),
      ban: () => acts?.askBan(it.author),
      spam: () => { void acts?.spam(it.author, it.id); },
    });
  });

  // ── The head, the strips, the foot ─────────────────────────────────────

  const pinnedPost = $derived(detail?.pinned.length ? (live.find((x) => x.id === detail.pinned[0]) ?? null) : null);

  async function showPinned() {
    const id = detail?.pinned[0];
    if (!id) return;
    // In the list shown: a list kept hidden may hold the same post.
    const el = scroller?.querySelector(`.list.on [id="post-${id}"]`);
    if (!el) return;
    el.scrollIntoView({ block: 'center', behavior: 'smooth' });
    highlighted = id;
    setTimeout(() => { if (highlighted === id) highlighted = null; }, 1600);
  }

  const members = $derived(row ? row.members : 0);
  const subtitle = $derived.by(() => {
    if (!row) return '';
    void $locale;
    const n = new Intl.NumberFormat(localeTag($locale)).format(members);
    // On a phone the head names the section shown: its button there is an icon alone.
    if (phone && shownScope) return `${$t('chan_section_sub', { title: scopeLabel(shownScope) })} · ${$t('chan_readers_n', { n })}`;
    return `${$t('chan_kind')} · ${$t('chan_readers_n', { n })}`;
  });

  const me_ = $derived(detail && me ? staffOf(detail, me) : null);
  const mutedText = $derived.by(() => {
    if (!detail || !amMuted(detail, me)) return null;
    const until = me_?.muted_until;
    const o = until ? opensAt(until, Math.floor(Date.now() / 1000), localeTag($locale)) : null;
    return o ? $t('chan_you_muted_until', { time: `${o.day ? `${o.day} ` : ''}${o.time}` }) : $t('chan_you_muted');
  });

  const queue = $derived(channelStore.queue[cid] ?? 0);
  const asks = $derived(channelStore.requests[cid] ?? 0);
  const levelAsks = $derived(channelStore.levelRequests[cid] ?? 0);
  /** What waits for me in the team (each to whom it is for). */
  const waitingQueue = $derived(isModerator(detail) ? queue : 0);
  const waitingAsks = $derived(answersRequests(detail) ? asks : 0);
  const waitingLevels = $derived(isModerator(detail) ? levelAsks : 0);
  /**
   * One count on ⋮, the rows in its menu (the redesign of the feed: no pills
   * in the head, on a computer either). Requests for a level have a bar over
   * the feed instead (ROO-259 item 3).
   */
  const waitingAll = $derived(waitingQueue + waitingAsks);

  let headMenu = $state<{ open: boolean; x: number; y: number }>({ open: false, x: 0, y: 0 });

  async function leave() {
    if (!row || !(await confirmStore.ask(tr('chan_leave_confirm', { name: row.name }), tr('chan_leave'), true))) return;
    await run(() => channelStore.act(cid, { op: 'leave' }));
  }

  async function forget() {
    if (!row || !(await confirmStore.ask(tr('chan_forget_confirm', { name: row.name }), tr('chan_forget'), true))) return;
    if (await run(() => channelStore.forget(cid))) onback?.();
  }

  /** While the key is awaited there is always a way out: no keeper may hold a reader forever. */
  async function cancelJoin() {
    if (!row || !(await confirmStore.ask(tr('chan_cancel_join_confirm', { name: row.name }), tr('chan_cancel_join'), true))) return;
    if (await run(() => channelStore.cancelJoin(cid))) onback?.();
  }

  // ── A keeper away ───────────────────────────────────────────────────────

  /** Now, a little behind: enough for «10 minutes without a key». */
  let nowS = $state(Math.floor(Date.now() / 1000));
  $effect(() => {
    const timer = setInterval(() => (nowS = Math.floor(Date.now() / 1000)), 30_000);
    return () => clearInterval(timer);
  });
  const waitingKey = $derived(where === 'waiting_key' || where === 'joining');
  /** The keeper said I was never let in: an invitation is needed, not a wait. */
  const needsInvite = $derived(waitingKey && !!detail?.keeper_refused);
  /** Waited too long for a key, and the keeper has not been heard of for as long (or ever). */
  const keeperAway = $derived(!!row && waitingKey && !needsInvite && keeperAwayAt(row, nowS));
  /** What the strip of a channel catching up says, by what its posts wait for. */
  const catchUp = $derived(detail ? catchUpNote(detail) : null);

  /** «Вам передают канал» (communities-wire §16): an offer of the owner, not answered yet. */
  const offerForMe = $derived(offerToMe(detail));

  async function answerTransfer(accept: boolean) {
    if (accept && !(await confirmStore.ask(tr('chan_transfer_accept_confirm'), tr('chan_transfer_accept'), false))) return;
    await run(async () => { await channelsApi.transfer.answer(cid, accept); await channelStore.refresh(cid); });
  }

  async function writeOwner() {
    if (row) await run(() => writeTo(row.owner));
  }

  const prefs = (fn: () => Promise<unknown>) => run(async () => { await fn(); channelStore.scheduleList(); });

  const headEntries = $derived.by((): MenuEntry[] => {
    if (!row) return [];
    void $locale;
    const r = row;
    return channelEntries(r, detail, waitingKey, tr, {
      info: () => openInfo(),
      moderation: () => openInfo(asks ? 'requests' : levelAsks ? 'levels' : 'queue'),
      pin: () => prefs(() => chatPrefs.setPinned(chatId, !r.pinned)),
      mute: () => prefs(() => chatPrefs.setMuted(chatId, !r.muted)),
      archive: () => prefs(() => chatPrefs.setArchived(chatId, !r.archived)),
      report: reportChannel,
      cancelJoin,
      leave,
      forget,
    }, {
      queue: waitingQueue, requests: waitingAsks,
      openQueue: () => openInfo('queue'), openRequests: () => openInfo('requests'),
    });
  });

  // The head's menu: its width, and its right edge under the ⋮'s.
  const HEAD_MENU_W = 288;
  /** The menu of a post: reactions on top, the moderator's pair side by side (the mockup c6.js, variant A). */
  const ITEM_MENU_W = 316;
  function openHeadMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    headMenu = { open: true, x: r.right - HEAD_MENU_W, y: r.bottom + 4 };
  }

  // The thread follows the post it is of: a new count, an edit, a removal.
  $effect(() => {
    const t0 = threadOf;
    if (!t0) return;
    const fresh = items.find((x) => x.id === t0.id);
    if (fresh?.deleted) untrack(() => { panel = null; threadOf = null; });
    else if (fresh && fresh !== t0) untrack(() => (threadOf = fresh));
  });

  /** I post here: the channel lets me, and the section shown takes my posts. */
  const posting = $derived(canPost(detail, me) && target !== null);
  /**
   * «Все» of a channel with sections has no composer (the owner's word,
   * 2026-10-23): one who may post somewhere is asked to open a section, and
   * the tiles open from here; one who may not sees what a reader sees.
   */
  const pickToPost = $derived(!posting && sectioned && scope === null && canPost(detail, me) && (where === 'reading' || where === 'catching_up'));
  const placeholder = $derived(!posting ? '' : targetScope ? $t('chan_compose_in', { title: scopeLabel(targetScope) }) : $t('chan_compose_post'));
  const files = $derived(canAttach(detail));
</script>

{#snippet quick()}
  <div class="quick msg-font">
    {#each QUICK as e (e)}
      <button class="q" class:mine={menu.item ? isMine(menu.item, e) : false}
        onclick={() => { const it = menu.item; menu.open = false; if (it) void react(it, e); }}>{e}</button>
    {/each}
  </div>
{/snippet}

{#snippet composerTools()}
  {#if files}<AttachButton onfiles={attach} surface="channel" {chatId} />{/if}
{/snippet}

{#snippet trayView()}
  <PickedTray picked={tray} />
{/snippet}

{#snippet stateNote(icon: string, title: string, text: string, tone: '' | 'mute' = '')}
  <div class="placeholder">
    <span class="ph-icon" class:mute={tone === 'mute'}><Icon name={icon} size={24} /></span>
    <p>{title}</p>
    <span>{text}</span>
  </div>
{/snippet}

{#snippet window()}
  <section class="window" class:phone>
    <header class="head" class:phone>
      {#if onback}
        {#if phone}<button class="icon back" onclick={onback} aria-label={$t('chan_back')}><Icon name="chevron-left" size={24} /></button>
        {:else}<button class="icon back narrow-only" onclick={onback} title={$t('chan_back')}><Icon name="arrow-left" size={18} /></button>{/if}
      {/if}
      <div class="ident" role="button" tabindex="0" onclick={() => (panel === 'info' ? (panel = null) : openInfo())}
        onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); openInfo(); } }}>
        <Avatar {...cover(row?.picture, cid)} label={row?.name ?? ''} seed={cid} size={phone ? 36 : 40} />
        <div class="who">
          <div class="title">
            <span class="t-name">{row?.name ?? $t('chan_kind')}</span>
            {#if row?.muted}<span class="dim"><Icon name="bell-off" size={14} /></span>{/if}
          </div>
          <div class="sub">
            {#if where === 'waiting_key' || where === 'joining'}<span class="warn-text">{$t('chan_state_joining')}</span>
            {:else if where === 'requested'}{$t('chan_state_requested')}
            {:else}{subtitle}{/if}
          </div>
        </div>
      </div>
      {#if sectioned && (where === 'reading' || where === 'catching_up')}
        {#if phone}
          <button bind:this={sectionButton} class="icon sec-icon" style:--chip={shownColor} onclick={() => (tilesOpen = !tilesOpen)} aria-haspopup="dialog" aria-expanded={tilesOpen}
            aria-label={$t('chan_sections_button', { title: shownName })} title={$t('chan_sections_button', { title: shownName })}><Icon name="layout-grid" size={20} /></button>
        {:else}
          <button bind:this={sectionButton} class="sec-btn" style:--chip={shownColor} onclick={() => (tilesOpen = !tilesOpen)} aria-haspopup="dialog" aria-expanded={tilesOpen}
            aria-label={$t('chan_sections_button', { title: shownName })} title={$t('chan_sections_button', { title: shownName })}>
            <SectionBadge scope={shownScope} communityId={cid} size={24} noPhoto />
            <span class="sec-name">{shownName}</span>
            <span class="sec-caret" class:up={tilesOpen}><Icon name="chevron-down" size={16} /></span>
          </button>
        {/if}
      {/if}
      <button class="icon menu-btn" class:on={headMenu.open} aria-expanded={headMenu.open} aria-haspopup="menu" onclick={openHeadMenu} title={$t('chan_menu')} aria-label={waitingAll > 0 ? `${$t('chan_menu')}: ${waitingAll}` : $t('chan_menu')}><Icon name="more-vertical" size={20} />
        {#if waitingAll > 0}<span class="menu-count" aria-hidden="true">{waitingAll > 99 ? '99+' : waitingAll}</span>{/if}</button>
    </header>
    <!-- Under the head, the colour of the section shown (the accent for «Все»). -->
    {#if sectioned && (where === 'reading' || where === 'catching_up')}<div class="stripe" style:background={shownColor} aria-hidden="true"></div>{/if}

    <!-- The bars under the head: cards the feed's width. -->
    <div class="strips">
      {#if locked}
        <div class="strip locked-note" role="status">
          <span class="s-icon"><Icon name="lock" size={16} /></span>
          <span class="strip-text"><b>{$t('chan_scope_locked')}.</b> {lockedText}</span>
          <button class="icon" onclick={() => (locked = null)} aria-label={$t('chan_close')} title={$t('chan_close')}><Icon name="x" size={18} /></button>
        </div>
      {/if}
      {#if pinnedPost && where === 'reading'}
        <button class="strip pinbar" onclick={showPinned}>
          <span class="s-icon"><Icon name="pin" size={16} /></span>
          <span class="pin-text"><b>{$t('chan_pinned')}</b> {pinnedPost.text ?? pinnedMedia(pinnedPost)}</span>
        </button>
      {/if}
      {#if row?.needs_update}
        <div class="strip warn"><span class="s-icon"><Icon name="download" size={16} /></span><span class="strip-text">{$t('chan_needs_update')}</span>
          <button class="btn btn-ghost btn-sm" onclick={() => void goto('/settings#updates')}>{$t('chan_update')}</button></div>
      {/if}
      {#if where === 'catching_up' && catchUp}
        <div class="strip" role="status"><span class="s-icon"><Icon name="key" size={16} /></span><span class="strip-text">{$t(countKey(catchUp.key, catchUp.n, $locale), { n: String(catchUp.n) })}</span></div>
      {/if}
      {#if offerForMe}
        <div class="strip transfer" role="status"><span class="s-icon"><Icon name="key" size={16} /></span>
          <span class="strip-text">{$t('chan_transfer_in', { name: nameStore.label(offerForMe.peer) })}</span>
          <button class="btn btn-primary btn-sm" onclick={() => answerTransfer(true)}>{$t('chan_transfer_accept')}</button>
          <button class="btn btn-ghost btn-sm" onclick={() => answerTransfer(false)}>{$t('chan_transfer_decline')}</button>
        </div>
      {/if}
      {#if waitingLevels > 0}
        <div class="strip" role="status"><span class="s-icon"><Icon name="layers" size={16} /></span>
          <span class="strip-text">{$t('chan_level_requests_bar', { n: String(waitingLevels) })}</span>
          <button class="btn btn-ghost btn-sm" onclick={() => openInfo('levels')}>{$t('chan_level_requests_open')}</button></div>
      {/if}
      {#if mutedText && joined(detail)}<div class="strip warn"><span class="s-icon"><Icon name="bell-off" size={16} /></span><span class="strip-text">{mutedText}</span></div>{/if}
      {#if error}<div class="strip danger" role="alert"><span class="s-icon"><Icon name="alert-triangle" size={16} /></span><span class="strip-text">{error}</span></div>{/if}
    </div>

    <div class="scroll" bind:this={scroller} {onscroll}>
      <div class="content">
        {#if missing}
          {@render stateNote('alert-triangle', $t('chan_missing'), $t('chan_missing_text'))}
        {:else if !detail}
          <div class="loading"><Icon name="loader" size={14} />{$t('chan_loading')}</div>
        {:else if waitingKey}
          <div class="placeholder">
            <span class="ph-icon away"><Icon name="key" size={24} /></span>
            {#if needsInvite}
              <p>{$t('chan_needs_invite')}</p>
              <span>{$t('chan_needs_invite_text')}</span>
            {:else if keeperAway && row}
              <p>{$t('chan_keeper_away')}</p>
              <span>{row.keeper_last_seen ? $t('chan_keeper_away_seen', { when: listStamp(row.keeper_last_seen) }) : $t('chan_keeper_away_text')}</span>
            {:else}
              <p>{$t('chan_waiting_key')}</p>
              <span>{$t('chan_waiting_key_text')}</span>
            {/if}
            <div class="ph-acts">
              {#if (keeperAway || needsInvite) && row?.owner !== me}<button class="btn btn-primary" onclick={writeOwner}><Icon name="send" size={18} />{$t('chan_write_owner')}</button>{/if}
              {#if !isOwner(detail)}<button class="btn btn-ghost" onclick={cancelJoin}><Icon name="x" size={18} />{$t('chan_cancel_join')}</button>{/if}
            </div>
          </div>
        {:else if where === 'requested'}
          {@render stateNote('clock', $t('chan_state_requested'), $t('chan_requested_text'))}
        {:else if where === 'rejected'}
          {@render stateNote('x', $t('chan_state_rejected'), $t('chan_rejected_text'))}
        {:else if where === 'disbanded'}
          {@render stateNote('archive', $t('chan_state_disbanded'), $t('chan_disbanded_text'), 'mute')}
        {:else if where === 'none'}
          {@render stateNote('lock-square-rounded', $t('chan_state_none'), $t('chan_none_text'))}
        {:else}
          {#if feedState === 'loading'}
            <div class="loading"><Icon name="loader" size={14} />{$t(settling ? 'chan_asking_relays' : 'chan_loading')}</div>
          {:else if feedState === 'scope_empty'}
            {@render stateNote('folder', $t('chan_scope_empty'), canPost(detail, me) ? $t('chan_no_posts_author') : $t('chan_no_posts_reader'))}
          {:else if feedState === 'empty'}
            {@render stateNote('speakerphone', $t('chan_no_posts'), canPost(detail, me) ? $t('chan_no_posts_author') : $t('chan_no_posts_reader'))}
          {:else if loadingEarlier}<div class="loading"><Icon name="loader" size={14} /></div>
          {:else if nextBefore}<div class="earlier"><button class="btn btn-ghost btn-sm" onclick={loadEarlier}><Icon name="arrow-up" size={13} />{$t('chan_load_earlier')}</button></div>
          {:else}<div class="history-start">{$t('chan_history_start')}</div>{/if}
          <!-- The lists drawn: the one shown, and those of the sections shown before it kept hidden, as they were. -->
          {#each drawn as k (k)}
            {@const all = unitsOf(k)}
            {@const ku = partial[k] !== undefined ? all.slice(-partial[k]) : all}
            {@const marked = sectioned && scopeOfKey(k) === null}
            <div class="list" class:on={k === key && feedState === 'feed'}>
              {#each ku as u, i (u.key)}
                {@const item = u.items[u.items.length - 1]}
                {@const sec = marked ? sectionOf(lv.scopes, item) : null}
                {#if i === 0 || dayOf(ku[i - 1].items[0].created_at) !== dayOf(u.items[0].created_at)}
                  {@const day = dayLabel(u.items[0].created_at)}
                  <div class="day"><span>{day.key ? $t(`chan_day_${day.key}` as 'chan_day_today') : day.text}</span></div>
                {/if}
                <FeedPost {item} parts={u.items.length > 1 ? u.items : null} detail={detail} highlighted={highlighted === item.id}
                  section={sec ? scopeLabel(sec) : null} sectionColor={sec ? lookOf(sec).color : null} sectionIcon={sec ? lookOf(sec).icon : null}
                  oncomments={detail.comments === 'on' || item.comments > 0 ? openThread : undefined} onread={openThread}
                  pending={u.items.some((x) => feedStore.isPending(x))} failure={feedStore.failure(item.id)} onretry={retryOutbox}
                  onreact={react} onmenu={(at, it) => (menu = { open: true, x: at.x, y: at.y, item: it })} />
              {/each}
            </div>
          {/each}
          {#if feedState === 'feed'}
            {#each uploads as u (u.id)}<UploadLine {u} />{/each}
            <!-- After what was read: why nothing new comes. -->
            {#if where === 'out' && row}<div class="out-note"><Icon name={row.out_reason === 'banned' ? 'ban' : 'lock-square-rounded'} size={13} />{$t(outKey(row, 'chan_out_text') as 'chan_out_text')}</div>{/if}
          {/if}
        {/if}
      </div>
    </div>

    <div class="bottom">
      <!-- Kept (hidden) in «Все»: a switch between a section and «Все» neither draws it anew nor loses its words. -->
      {#if posting || pickToPost}
        <div class="compose" class:off={!posting}>
          <Composer bind:this={composer} replyTo={null} editing={null} peerTitle={row?.name ?? ''} {placeholder} oncancel={() => {}} onsend={send} tools={composerTools}
            markdown onexpand={expand}
            draftKey={chatId} autofocus={!phone} onrecording={record} canRecord={files} attached={tray.files.length ? 'ready' : null}
            attachments={tray.files.length ? trayView : undefined} canattach={files} />
        </div>
      {/if}
      {#if posting}
        <!-- The composer above. -->
      {:else if pickToPost}
        <div class="no-composer"><div class="nc-in to-section">
          <span class="nc-hint"><Icon name="layout-grid" size={16} /><span>{$t('chan_open_section_to_post')}</span></span>
          <button class="btn btn-ghost" onclick={() => (tilesOpen = true)}>{$t('chan_sections')}</button>
        </div></div>
      {:else if where === 'reading' || where === 'catching_up'}
        <div class="no-composer"><div class="nc-in">
          <span class="nc-hint"><Icon name={canComment(detail, me) ? 'message-circle' : 'lock-square-rounded'} size={16} />
            <span>{canComment(detail, me) ? $t('chan_readers_comment') : mutedText ?? $t('chan_readers_read')}</span></span>
        </div></div>
      {:else if where && where !== 'waiting_key' && where !== 'joining'}
        <div class="no-composer"><div class="nc-in">
          <button class="btn btn-ghost" onclick={forget}><Icon name="trash-2" size={18} />{$t('chan_forget')}</button>
        </div></div>
      {/if}
    </div>
  </section>
{/snippet}

{#snippet side()}
  {#if panel === 'thread' && threadOf && detail}
    {@const look = sectionOf(lv.scopes, threadOf)}
    <ThreadView {detail} post={threadOf} parts={albumOf.get(threadOf.id) ?? null} section={sectionTitle(lv.scopes, threadOf)}
      sectionColor={look ? lookOf(look).color : null} sectionIcon={look ? lookOf(look).icon : null}
      onedit={openEdit} onclose={() => { panel = null; threadOf = null; }} />
  {:else if panel === 'info' && detail}
    <ChannelInfo {detail} bind:moderation bind:modTab onclose={() => (panel = null)} onforgotten={() => { panel = null; onback?.(); }} />
  {/if}
{/snippet}

<!-- The extended editor lies over the whole pane; the feed under it stays as it was (its place, its composer), out of sight. -->
<div class="host">
  <div class="under" class:covered={!!editor} inert={!!editor}>
    <WithInfo open={!!panel && !!detail} chat={window} info={side} prefKey="channels.side" />
  </div>
  {#if editor && detail}
    <PostEditor {chatId} {detail} draft={editor.draft} section={editorSection} item={editor.item} parts={editor.item ? (albumOf.get(editor.item.id) ?? null) : null}
      picked={editor.item ? null : tray} canAttach={files} onpublish={publishEdited} onclose={closeEditor} />
  {/if}
</div>
<MediaViewer />
<ContextMenu bind:open={menu.open} x={menu.x} y={menu.y} items={entries} header={menu.item && !menu.item.deleted && joined(detail) ? quick : undefined}
  width="{ITEM_MENU_W}px" onclose={() => (menu.open = false)} />
{#if detail}<PersonActions bind:this={acts} {detail} onerror={(e) => (error = e)} />{/if}
{#if detail}<LevelRequestDialog bind:open={asking.open} {detail} preselect={asking.level} />{/if}
{#if detail && sectioned}
  <SectionTiles bind:open={tilesOpen} {tiles} communityId={cid} current={scope} unread={row?.unread ?? 0} anchor={sectionButton}
    onpick={pick} onlocked={onLocked} onsettings={canSetUpSections(detail) ? setUpSections : undefined} />
{/if}
{#if detail && canSetUpSections(detail)}<ScopeDialog bind:open={scopeDialog.open} {detail} scopeId={scopeDialog.id} />{/if}
<ContextMenu bind:open={headMenu.open} x={headMenu.x} y={headMenu.y} items={headEntries} width="{HEAD_MENU_W}px" onclose={() => (headMenu.open = false)} />

<style>
  .host { position: relative; height: 100%; min-height: 0; display: flex; flex-direction: column; }
  .under { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .under > :global(*) { flex: 1; min-height: 0; }
  .under.covered { visibility: hidden; }
  .window { position: relative; display: flex; flex-direction: column; height: 100%; min-height: 0; container-type: inline-size; }
  /* The head (the redesign of the feed): who on the left, its name never gives way; the section, ⋮ with one count. */
  .head { display: flex; align-items: center; gap: 10px; padding: var(--sp-3) var(--sp-4) var(--sp-3) var(--sp-5); border-bottom: 1px solid var(--pane-border); background: var(--pane-head); flex-shrink: 0; min-height: 68px; box-sizing: border-box; }
  /* A phone and a tablet: the whole screen, the head a bar. */
  /* Over the feed's own layers (a post's ⋮ on its media), under the info panel that lies over the conversation (WithInfo):
     no layer of the scale sits there, so the base's plus two. */
  .head.phone { position: relative; z-index: calc(var(--z-base) + 2); min-height: 60px; gap: 6px; padding: var(--sp-2) var(--sp-2) var(--sp-2) var(--sp-1); background: var(--glass-bar); -webkit-backdrop-filter: var(--glass-filter); backdrop-filter: var(--glass-filter); }
  .head.phone .back { color: var(--text); }
  .ident { display: flex; align-items: center; gap: var(--sp-3); min-width: 160px; flex: 1 1 auto; cursor: pointer; border-radius: var(--radius-sm); }
  .head.phone .ident { min-width: 0; gap: 10px; }
  .ident:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  .who { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
  .title { display: flex; align-items: center; gap: 6px; font-weight: var(--fw-bold); font-size: var(--fs-md); overflow: hidden; white-space: nowrap; }
  .t-name { overflow: hidden; text-overflow: ellipsis; min-width: 0; }
  .sub { font-size: var(--fs-sm); color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .warn-text { color: var(--warn-text); }
  .dim { color: var(--text-3); display: inline-flex; }
  .icon { border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; padding: 6px; border-radius: var(--radius-sm); }
  .icon:hover { color: var(--text); background: var(--surface-3); }
  .icon:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  .head .icon { flex-shrink: 0; width: 36px; height: 36px; padding: 0; align-items: center; justify-content: center; border-radius: var(--radius); }
  .head.phone .icon { width: 40px; height: 40px; border-radius: var(--radius-md); }
  .narrow-only { display: none; }
  @media (max-width: 860px) { .head .narrow-only { display: inline-flex; } }
  /* The button of the sections: the mark and the name of the section shown, the caret. */
  .sec-btn {
    display: inline-flex; align-items: center; gap: var(--sp-2); height: 36px; max-width: 260px; padding: 0 10px 0 6px; flex-shrink: 0; min-width: 0;
    border: none; border-radius: var(--radius-md); background: none; cursor: pointer; font: inherit; font-size: var(--fs-base); font-weight: var(--fw-semibold); color: var(--text);
    transition: background var(--dur-fast);
  }
  .sec-btn:hover, .sec-btn[aria-expanded="true"] { background: var(--surface-2); }
  .sec-btn:focus-visible, .sec-icon:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  .sec-name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sec-caret { display: inline-flex; color: var(--text-3); transition: transform var(--dur-fast); }
  .sec-caret.up { transform: rotate(180deg); }
  /* On a phone an icon alone, beside the menu, in the ink of the section's colour. */
  .sec-icon.icon { color: color-mix(in srgb, var(--chip) 65%, var(--text)); }
  .sec-icon.icon[aria-expanded="true"] { background: var(--surface-2); }
  .stripe { height: 3px; flex-shrink: 0; }
  .menu-btn { position: relative; }
  /* While its menu is open the ⋮ stays lit. */
  .menu-btn.on { background: var(--surface-2); color: var(--text); }
  /* The count on ⋮ is the primitive .menu-count of base.css; on a phone it sits a little lower. */
  .head.phone .menu-count { top: 0; right: -2px; }
  /* The bars under the head: cards the feed's width. */
  .strips { display: flex; flex-direction: column; gap: var(--sp-2); padding: var(--sp-3) var(--sp-6) 0; flex-shrink: 0; }
  .strips:not(:has(> *)) { display: none; }
  .strip {
    display: flex; align-items: center; gap: 10px; width: 100%; max-width: 720px; margin: 0 auto; min-height: 44px; box-sizing: border-box;
    padding: var(--sp-2) var(--sp-2) var(--sp-2) var(--sp-3); border-radius: var(--radius-lg); border: 1px solid var(--bubble-in-border); background: var(--bubble-in-bg);
    color: var(--text-2); font: inherit; font-size: var(--fs-sm); line-height: 1.45; text-align: left;
  }
  .s-icon { flex-shrink: 0; width: 28px; height: 28px; border-radius: var(--radius-sm); display: inline-flex; align-items: center; justify-content: center; background: var(--accent-bg); color: var(--accent-text); }
  .strip-text { flex: 1; min-width: 0; }
  .strip .icon { flex-shrink: 0; }
  .locked-note b { font-weight: var(--fw-semibold); color: var(--text-body); }
  .pinbar { cursor: pointer; }
  .pinbar:hover { background: var(--surface-hover); }
  .strip.pinbar:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  .pin-text { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-2); }
  .pin-text b { color: var(--accent-text); font-weight: var(--fw-bold); margin-right: 4px; }
  .strip.warn { color: var(--text); background: var(--warn-bg); border-color: var(--warn-border); }
  .strip.warn .s-icon { background: var(--warn-bg); color: var(--warn-text); }
  .strip.danger { color: var(--danger-text); background: var(--danger-bg); border-color: var(--danger-border); }
  .strip.danger .s-icon { background: var(--danger-bg); color: var(--danger-text); }
  /* The feed: a column of cards. */
  .scroll { flex: 1; min-height: 0; overflow-y: auto; padding: var(--sp-3) var(--sp-6) var(--sp-4); display: flex; flex-direction: column; }
  .content { margin-top: auto; display: flex; flex-direction: column; gap: var(--sp-3); flex-shrink: 0; }
  .content:has(> .placeholder) { margin-bottom: auto; }
  /* The states without a feed. */
  .placeholder { margin: auto; display: flex; flex-direction: column; align-items: center; gap: 10px; color: var(--text-2); text-align: center; padding: var(--sp-6); }
  .ph-icon { width: 56px; height: 56px; margin-bottom: 6px; border-radius: var(--radius-lg); display: inline-flex; align-items: center; justify-content: center; background: var(--accent-bg); color: var(--accent-text); }
  .placeholder p { margin: 0; color: var(--text); font-weight: var(--fw-bold); font-size: var(--fs-lg); }
  .placeholder span { font-size: var(--fs-base); max-width: 380px; line-height: 1.55; }
  .ph-icon.away { background: var(--warn-bg); color: var(--warn-text); }
  .ph-icon.mute { background: var(--surface-3); color: var(--text-3); }
  .ph-acts { display: flex; flex-wrap: wrap; justify-content: center; gap: var(--sp-2); margin-top: var(--sp-2); }
  .loading { display: flex; justify-content: center; align-items: center; gap: var(--sp-2); color: var(--text-3); padding: var(--sp-2); font-size: var(--fs-xs); }
  .loading :global(svg) { animation: spin 1.1s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .earlier { display: flex; justify-content: center; }
  .history-start { text-align: center; color: var(--text-3); font-size: var(--fs-xs); padding: var(--sp-2) var(--sp-1) 0; }
  .out-note { align-self: center; display: inline-flex; align-items: center; gap: 6px; text-align: center; line-height: 1.4; font-size: var(--fs-xs); color: var(--text-2); padding: var(--sp-1) var(--sp-3); border-radius: var(--radius-pill); background: var(--surface-2); }
  /* A list kept hidden takes no place; the one shown lays its cards out as children of the feed. */
  .list { display: none; }
  .list.on { display: contents; }
  .day { display: flex; justify-content: center; }
  .day span { display: inline-flex; align-items: center; height: 26px; box-sizing: border-box; font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--text-2); padding: 0 var(--sp-3); border-radius: var(--radius-pill); background: var(--surface-3); border: 1px solid var(--border); }
  .bottom { flex-shrink: 0; }
  .compose.off { display: none; }
  /* The foot without a composer: what readers do, or the way to a section, the feed's width. */
  .no-composer { padding: var(--sp-3) var(--sp-6); border-top: 1px solid var(--pane-border); }
  .nc-in { max-width: 720px; margin: 0 auto; min-height: 44px; display: flex; align-items: center; justify-content: center; gap: var(--sp-3); }
  .nc-in.to-section { justify-content: space-between; }
  .nc-hint { display: flex; align-items: center; gap: var(--sp-2); min-width: 0; font-size: var(--fs-base); color: var(--text-3); }
  .nc-hint :global(svg) { flex-shrink: 0; }
  .no-composer .btn { min-height: var(--control-h); flex-shrink: 0; }
  .window.phone .no-composer .btn { min-height: var(--control-h-lg); }
  /* A narrow window: the name keeps its room, the section is its mark alone. */
  @container (max-width: 620px) {
    .head:not(.phone) .sec-name, .head:not(.phone) .sec-caret { display: none; }
    .head:not(.phone) .sec-btn { width: 36px; padding: 0; justify-content: center; }
    .window:not(.phone) .strips, .window:not(.phone) .scroll, .window:not(.phone) .no-composer { padding-left: var(--sp-4); padding-right: var(--sp-4); }
  }
  /* A phone (a tablet keeps the computer's room around the cards). */
  @container (max-width: 599px) {
    .window.phone .strips { padding: var(--sp-2) var(--sp-3) 0; }
    .window.phone .scroll { padding: var(--sp-2) var(--sp-3) var(--sp-3); }
    .window.phone .content { gap: 10px; }
    .window.phone .no-composer { padding: 10px var(--sp-3) 14px; }
  }
  /* The reactions over the menu of a post: round 40, spread across it (the mockup c6.js). */
  .quick { display: flex; justify-content: space-between; gap: var(--sp-1); padding: var(--sp-1) var(--sp-2); }
  /* The circles shrink to fit any width of the menu (40 at most, 44 on a finger), evenly spread, none cut. */
  .q { flex: 1 1 0; min-width: 0; max-width: 40px; aspect-ratio: 1; height: auto; padding: 0; border: none; background: var(--surface-3); font-size: 20px; line-height: 1; border-radius: 50%; cursor: pointer; display: grid; place-items: center; }
  @media (pointer: coarse) { .q { max-width: 44px; } }
  .q:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  .q:hover { background: var(--surface-hover); }
  .q.mine { background: var(--accent-bg); }
  @media (pointer: coarse) {
    .icon { padding: 10px; }
  }
</style>
