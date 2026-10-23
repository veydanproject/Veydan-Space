<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The page of a post, beside the feed (over it on a phone): the whole post
  on top (its title, every part of an album, its files, all its words, its
  reactions, who wrote it and when it was edited), then its comments
  oldest first, each answer under the comment it answers, earlier ones a
  page at a time, and the field to comment or to answer at the bottom.
  «Читать» and «Комментарии» of a post in the feed open it. Anyone in the
  channel comments while the channel takes comments; a file goes with a
  comment for authors and above.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { tick, untrack } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { copied, toast } from '$lib/core/ui/toast.svelte';
  import ContextMenu, { type MenuEntry } from '$lib/core/ui/ContextMenu.svelte';
  import { AttachButton, Composer, confirmStore, myKey, onPhone, reportStore, writeTo } from '$lib/messenger/seams/shared';
  import UploadLine from './UploadLine.svelte';
  import { uploadsUnder } from './uploads';
  import FeedPost from './FeedPost.svelte';
  import PickedTray from './PickedTray.svelte';
  import { Picked } from './picked.svelte';
  import { channelStore } from './channelStore.svelte';
  import { nameStore } from './names.svelte';
  import { channelsApi, type ChannelDetail, type ChannelRecording, type FeedItemView, type TextFormat } from './api';
  import { channelError } from './errors';
  import { itemEntries } from './itemMenu';
  import { feedStore, isLocal, localId, threadKey } from './feedStore.svelte';
  import { isMine, QUICK } from './reactions';
  import { canAttach, canComment, canRecord } from './permissions';
  import { threadOrder } from './thread';
  import PersonActions from './moderation/PersonActions.svelte';

  interface Props {
    detail: ChannelDetail;
    post: FeedItemView;
    /** The posts of the album the post ends, oldest first: every part is shown. */
    parts?: FeedItemView[] | null;
    /** The title of the post's section, when it is not everyone's. */
    section?: string | null;
    /** «Изменить» in the menu of the post (the dialog is the feed's). */
    onedit?: (item: FeedItemView) => void;
    onclose: () => void;
  }
  let { detail, post, parts = null, section = null, onedit, onclose }: Props = $props();

  const phone = onPhone();
  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);
  const PAGE = 50;
  const cid = $derived(detail.channel.id);
  const me = $derived(myKey());

  /** The comments as read on this device (feedStore): a thread opened again shows them at once. */
  const key = $derived(threadKey(cid, post.id));
  const list = $derived(feedStore.list(key));
  const comments = $derived(feedStore.shown(key));
  const nextBefore = $derived(list?.next);
  let loadingEarlier = $state(false);
  let error = $state('');
  /** «Loading» only until the first page of this thread was read. */
  const loading = $derived(!list?.loaded && !error);
  let replyTo = $state<FeedItemView | null>(null);
  let scroller = $state<HTMLDivElement | null>(null);

  async function load() {
    const id = cid;
    const pid = post.id;
    const k = key;
    const keep = stick;
    try {
      await feedStore.refresh(k, () => channelsApi.thread(id, pid, undefined, PAGE));
      if (k === key) error = '';
    } catch (e) {
      if (k === key) error = channelError(e, tr);
    }
    // The page opens at the post (its top); the newest comment is kept in sight once it was scrolled to.
    if (k === key && keep) await scrollDown();
  }

  async function scrollDown() {
    await tick();
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  }

  /** The page above, the view kept on what was read. */
  async function earlier() {
    if (!nextBefore || loadingEarlier) return;
    loadingEarlier = true;
    const el = scroller;
    const from = el ? el.scrollHeight - el.scrollTop : 0;
    const id = cid;
    const pid = post.id;
    try {
      if (!(await feedStore.earlier(threadKey(id, pid), (next) => channelsApi.thread(id, pid, next, PAGE)))) return;
      await tick();
      if (el) el.scrollTop = el.scrollHeight - from;
    } catch (e) {
      error = channelError(e, tr);
    } finally {
      loadingEarlier = false;
    }
  }

  /** Scrolled down to the newest comment: new ones keep it there. */
  let stick = false;
  function onscroll() {
    const el = scroller;
    if (el) stick = el.scrollTop > 0 && el.scrollHeight - el.scrollTop - el.clientHeight < 80;
  }

  let shownFor = '';
  $effect(() => {
    const pid = post.id;
    void channelStore.revision[cid];
    if (pid !== shownFor) {
      shownFor = pid;
      untrack(() => { replyTo = null; error = ''; tray.clear(); stick = false; if (scroller) scroller.scrollTop = 0; });
    }
    untrack(() => load());
  });

  // The thread on the screen is not trimmed meanwhile; left, it keeps its newest comments.
  $effect(() => {
    const k = key;
    untrack(() => feedStore.enter(k));
    return () => feedStore.leave(k, null);
  });

  const ordered = $derived(threadOrder(comments));
  const byId = $derived(new Map(comments.map((c) => [c.id, c])));

  async function run(fn: () => Promise<unknown>): Promise<boolean> {
    error = '';
    try { await fn(); return true; }
    catch (e) { error = channelError(e, tr); return false; }
  }

  /** `fmt`: the composer's Markdown menu was used. */
  /**
   * A comment of mine stands in the thread at once and the post counts it
   * at once; a refusal takes both back and puts the words into the composer.
   */
  async function send(text: string, fmt: TextFormat | null = null) {
    const body = text.trim();
    if (tray.files.length) return sendFiles(body, fmt);
    if (!body) return;
    const id = cid;
    const pid = post.id;
    const k = key;
    const parent = replyTo?.id ?? null;
    const temp = localId();
    error = '';
    feedStore.addLocal(k, {
      id: temp, community_id: id, post_id: pid, parent_id: parent ?? undefined, author: me ?? '', created_at: Math.floor(Date.now() / 1000), text: body,
      fmt: fmt ?? undefined, review: 'ok', hidden: false, deleted: false, pinned: false, reactions: [], comments: 0, pending: true,
    });
    feedStore.bumpComments(id, pid, 1);
    replyTo = null;
    void scrollDown();
    try {
      feedStore.settle(k, temp, await channelsApi.comment(id, pid, body, parent, fmt));
    } catch (e) {
      feedStore.dropLocal(k, temp);
      feedStore.bumpComments(id, pid, -1);
      error = channelError(e, tr);
      throw new Error(error);
    }
    void load();
  }

  async function retryOutbox() {
    await run(() => channelStore.retryOutbox(cid));
  }

  /** Picked files wait above the composer for the words that go with them; several go as one album. */
  const tray = new Picked();
  function attach(paths: string[]) {
    tray.add(paths);
  }

  async function sendFiles(caption: string, fmt: TextFormat | null = null) {
    const id = cid;
    const pid = post.id;
    const parent = replyTo?.id ?? null;
    error = '';
    try {
      await tray.send((path, opts) => channelsApi.mediaSend(id, path, { postId: pid, parent, ...opts }), caption, fmt);
      replyTo = null;
    } catch (e) {
      error = channelError(e, tr);
      throw new Error(error);
    } finally {
      await channelStore.refreshUploads(id).catch(() => {});
    }
  }

  /**
   * A voice message or a circle recorded in the composer goes as a comment
   * (an answer to the comment chosen), as a picked file does; a reader who
   * may comment records too (2026-10-23). Its line stands under the
   * comments until it is one.
   */
  async function record(rec: ChannelRecording) {
    const id = cid;
    const pid = post.id;
    const parent = replyTo?.id ?? null;
    error = '';
    stick = true;
    try {
      await channelsApi.mediaRecord(id, rec, { postId: pid, parent });
      replyTo = null;
    } catch (e) {
      error = channelError(e, tr);
      throw new Error(error);
    } finally {
      await channelStore.refreshUploads(id).catch(() => {});
      void scrollDown();
    }
  }

  /** My files on their way under this post. */
  const uploads = $derived(uploadsUnder(Object.values(channelStore.uploads), cid, post.id));

  /** Toggled on the tap; a refusal takes it back and says why. */
  async function react(item: FeedItemView, emoji: string) {
    const id = cid;
    await run(() => feedStore.react(id, item, emoji, (set) => channelsApi.react(id, item.id, emoji, set)));
  }

  async function remove(item: FeedItemView) {
    const mineIt = item.author === me;
    if (!(await confirmStore.ask(tr(mineIt ? 'chan_delete_confirm' : 'chan_delete_other_confirm', { name: nameStore.label(item.author) }), tr('chan_menu_delete'), true))) return;
    // The post of an album goes whole, as from the feed.
    const all = item.id === post.id && parts?.length ? parts : [item];
    const id = cid;
    await run(async () => { for (const p of all) await channelsApi.remove(id, p.id); });
  }

  let menu = $state<{ open: boolean; x: number; y: number; item: FeedItemView | null }>({ open: false, x: 0, y: 0, item: null });
  /** A moderator's acts on the author of a comment, asked as from the team. */
  let acts = $state<PersonActions | null>(null);
  const entries = $derived.by((): MenuEntry[] => {
    const it = menu.item;
    if (!it) return [];
    return itemEntries(it, detail, me, tr, {
      reply: it.post_id ? () => (replyTo = it) : undefined,
      copy: () => { navigator.clipboard.writeText(it.text ?? '').then(() => copied()).catch(() => {}); },
      edit: onedit && !it.post_id ? () => onedit(it) : undefined,
      remove: () => { void remove(it); },
      hide: () => { void run(() => channelsApi.moderation.mod(cid, { t: 'hide', targets: [it.id] })); },
      approve: () => { void run(() => channelsApi.moderation.mod(cid, { t: 'approve', target: it.id, version: it.id })); },
      reject: () => { void run(() => channelsApi.moderation.mod(cid, { t: 'reject', target: it.id })); },
      write: () => { writeTo(it.author).catch((e) => toast.error(channelError(e, tr))); },
      report: () => reportStore.open({ accused: it.author, name: nameStore.label(it.author), context: { t: 'channel', community_id: cid, post_id: it.id }, item: it.post_id ? 'comment' : 'post', canBlock: false }),
      hideAuthor: () => acts?.askHide(it.author),
      mute: () => acts?.askMute(it.author),
      ban: () => acts?.askBan(it.author),
      spam: () => { void acts?.spam(it.author, it.id); },
    });
  });

  const can = $derived(canComment(detail, me));
  /** As many as the post counts, or as are read here when more. */
  const commentCount = $derived(Math.max(post.comments, comments.length));

  function postMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu = { open: true, x: r.right - 220, y: r.bottom + 4, item: post };
  }
  const placeholder = $derived($t('chan_compose_comment'));
</script>

{#snippet quick()}
  <div class="quick msg-font">
    {#each QUICK as e (e)}
      <button class="q" class:mine={menu.item ? isMine(menu.item, e) : false}
        onclick={() => { const it = menu.item; menu.open = false; if (it) void react(it, e); }}>{e}</button>
    {/each}
  </div>
{/snippet}

{#snippet tools()}
  {#if canAttach(detail)}<AttachButton onfiles={attach} />{/if}
{/snippet}

{#snippet trayView()}
  <PickedTray picked={tray} />
{/snippet}

<div class="panel">
  <!-- The bar of the page: «Пост», on a phone with its section and channel under it; the menu of the post; the way back. -->
  <header class="phead" class:phone>
    {#if phone}<button class="icon back" onclick={onclose} aria-label={$t('chan_back')}><Icon name="chevron-left" size={24} /></button>{/if}
    <div class="ph-titles">
      <span class="ph-title">{$t('chan_post_page')}</span>
      {#if phone}<span class="ph-sub">{section ? `${section} · ` : ''}{detail.channel.name}</span>{/if}
    </div>
    {#if !isLocal(post.id)}
      <button class="icon" onclick={postMenu} aria-label={$t('chan_post_menu')} title={$t('chan_post_menu')}><Icon name="more-vertical" size={phone ? 20 : 16} /></button>
    {/if}
    {#if !phone}<button class="icon" onclick={onclose} aria-label={$t('chan_close')} title={$t('chan_close')}><Icon name="x" size={16} /></button>{/if}
  </header>
  <div class="scroll" bind:this={scroller} {onscroll}>
    <FeedPost item={post} {parts} {detail} full section={phone ? null : section} onreact={react} onmenu={(at, it) => (menu = { open: true, x: at.x, y: at.y, item: it })} />
    <div class="label">{$t('chan_comments')}{#if commentCount}{' · '}{commentCount}{/if}</div>
    {#if loadingEarlier}<div class="loading"><Icon name="loader" size={14} /></div>
    {:else if nextBefore}<button class="btn btn-ghost btn-sm earlier" onclick={earlier}><Icon name="arrow-up" size={13} />{$t('chan_load_earlier')}</button>{/if}
    {#if loading}
      <div class="loading"><Icon name="loader" size={14} /></div>
    {:else if !comments.length}
      <p class="empty">{detail.comments === 'on' ? $t('chan_comments_empty') : $t('chan_comments_off')}</p>
    {/if}
    {#each ordered as c (c.item.id)}
      <div class="row" style:--depth={c.depth}>
        <FeedPost item={c.item} {detail} comment
          replyTo={c.item.parent_id ? nameStore.label(byId.get(c.item.parent_id)?.author ?? '') : null}
          pending={feedStore.isPending(c.item)} failure={feedStore.failure(c.item.id)} onretry={retryOutbox}
          onreact={react} onreply={can ? (it) => (replyTo = it) : undefined} onmenu={(at, it) => (menu = { open: true, x: at.x, y: at.y, item: it })} />
      </div>
    {/each}
    {#each uploads as u (u.id)}<UploadLine {u} />{/each}
    {#if error}<div class="error-msg">{error}</div>{/if}
  </div>
  {#if can}
    {#if replyTo}
      <div class="replying">
        <Icon name="reply" size={13} />
        <span class="r-text">{$t('chan_replying_to', { name: nameStore.label(replyTo.author) })}</span>
        <button class="x" onclick={() => (replyTo = null)} aria-label={$t('chan_cancel')}><Icon name="x" size={13} /></button>
      </div>
    {/if}
    <Composer replyTo={null} editing={null} peerTitle={detail.channel.name} {placeholder} oncancel={() => (replyTo = null)} onsend={send} {tools}
      draftKey={`${detail.channel.chat_id}#${post.id}`} autofocus={!phone} onrecording={record} canRecord={canRecord(detail, me, 'comment')} markdown attached={tray.files.length ? 'ready' : null}
      attachments={tray.files.length ? trayView : undefined} canattach={canAttach(detail)} />
  {:else}
    <div class="no-composer"><Icon name="lock-square-rounded" size={13} />{detail.comments === 'on' ? $t('chan_comments_closed_for_me') : $t('chan_comments_off')}</div>
  {/if}
</div>

<ContextMenu bind:open={menu.open} x={menu.x} y={menu.y} items={entries} header={menu.item && !menu.item.deleted ? quick : undefined} onclose={() => (menu.open = false)} />
<PersonActions bind:this={acts} {detail} onerror={(e) => (error = e)} />

<style>
  .panel { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--surface); }
  .phead { display: flex; align-items: center; gap: var(--sp-1); min-height: 56px; padding: var(--sp-2) var(--sp-2) var(--sp-2) var(--sp-4); border-bottom: 1px solid var(--border); flex-shrink: 0; }
  .ph-titles { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .ph-title { font-weight: var(--fw-bold); font-size: var(--fs-base); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ph-sub { font-size: var(--fs-2xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .icon { border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; padding: 6px; border-radius: var(--radius-sm); }
  .icon:hover { color: var(--text); background: var(--surface-3); }
  /* On a phone the bar floats inset, as the head of the channel over its feed. */
  .phead.phone {
    position: relative; z-index: 2; margin: var(--sp-2) var(--sp-2) 0; min-height: 56px; padding: 0 var(--sp-1);
    border: 1px solid var(--pane-strong-border); border-radius: var(--radius-head);
    background: var(--pane-strong); box-shadow: var(--pane-highlight), var(--pane-shadow);
    -webkit-backdrop-filter: var(--pane-filter); backdrop-filter: var(--pane-filter);
  }
  .phead.phone .ph-title { font-size: var(--fs-md); font-weight: var(--fw-semibold); }
  .phead.phone .back { color: var(--text); padding: 10px; }
  .phead.phone .icon { padding: 10px; }
  .panel { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--surface); }
  .scroll { flex: 1; min-height: 0; overflow-y: auto; padding: var(--sp-3) var(--sp-4); display: flex; flex-direction: column; gap: var(--sp-2); }
  .label { margin-top: var(--sp-2); font-size: var(--fs-2xs); text-transform: uppercase; letter-spacing: 0.6px; color: var(--text-3); font-weight: var(--fw-bold); }
  .row { padding-left: calc(min(var(--depth), 3) * 16px); }
  .earlier { align-self: center; }
  .empty { margin: 0; font-size: var(--fs-sm); color: var(--text-3); text-align: center; padding: var(--sp-4) 0; }
  .loading { display: flex; justify-content: center; color: var(--text-3); padding: var(--sp-2); }
  .replying { display: flex; align-items: center; gap: 6px; padding: 6px var(--sp-3); border-top: 1px solid var(--border); font-size: var(--fs-xs); color: var(--accent-text-2); background: var(--surface-2); }
  .r-text { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .x { border: none; background: none; color: var(--text-3); cursor: pointer; display: inline-flex; padding: 4px; }
  .no-composer { display: flex; align-items: center; justify-content: center; gap: 6px; padding: var(--sp-3); border-top: 1px solid var(--border); font-size: var(--fs-xs); color: var(--text-3); }
  .quick { display: flex; gap: 2px; padding: 2px; }
  .q { border: none; background: none; font-size: 20px; width: 36px; height: 36px; border-radius: var(--radius-sm); cursor: pointer; }
  .q:hover { background: var(--surface-hover); }
  .q.mine { background: var(--accent-bg); }
</style>
