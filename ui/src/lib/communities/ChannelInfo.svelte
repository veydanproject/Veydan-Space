<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->
<!-- Copied from ui/src/lib/messenger/groups/GroupInfo.svelte at 8ce09c39, then changed for channels. -->

<!--
  Everything about a channel except its feed: its card and rules, the
  keeper that hands out its keys, the link, the team (owner, admins,
  moderators, authors) and those the journal mutes or trusts, the banned,
  and the way out. Its moderation is a mode of the same panel. The readers
  are listed to the team alone (ChannelReaders: the keeper's rows or its
  snapshot, as the journal does not know them); a reader sees none.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { tick, untrack } from 'svelte';
  import { pushState } from '$app/navigation';
  import { page } from '$app/state';
  import { countKey, locale, localeTag, t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import ContextMenu, { type MenuEntry } from '$lib/core/ui/ContextMenu.svelte';
  import { Avatar, MessageContent, PanelHead, confirmStore, isTauriHost, listStamp, longpress, myKey, onPhone, reportStore, serverStore, stamp, writeTo } from '$lib/messenger/seams/shared';
  import ChannelLink from './ChannelLink.svelte';
  import ChannelReaders from './ChannelReaders.svelte';
  import InvitePicker from './InvitePicker.svelte';
  import { channelStore } from './channelStore.svelte';
  import { nameStore, shortKey } from './names.svelte';
  import { channelsApi, type ChannelAction, type ChannelDetail, type KeeperStatus, type StaffView } from './api';
  import { channelError } from './errors';
  import { keeperBackground, keeperMove } from './keeper';
  import { canTransferTo, myOffer } from './transfer';
  import { HISTORIES, JOINS, outKey, rulesLines } from './lines';
  import { cover } from './cover';
  import {
    KEEPER_AWAY_SECS, canBan, canInvite, canMute, canRemove, canRotateKey, canTrust, canUnban, checksMessages, isManager, isModerator, isOwner, joined,
    canSetLevels, mutedNow, rolesFor, staffOf, type Role,
  } from './permissions';
  import ModerationPanel, { type ModerationTab } from './moderation/ModerationPanel.svelte';
  import PersonActions from './moderation/PersonActions.svelte';
  import { opensAt, weekdayNames } from './moderation/schedule';
  import LevelsEditor from './levels/LevelsEditor.svelte';
  import MemberLevelsDialog from './levels/MemberLevelsDialog.svelte';
  import MyLevels from './levels/MyLevels.svelte';
  import { EVERYONE, levelNames, levelsOf, levelsOfPerson } from './levels/sections';
  import { orderedLevels } from './levels/editor';
  import { ordered } from './levels/look';
  import NavRow from '$lib/core/ui/NavRow.svelte';
  import Box from './info/Box.svelte';
  import Tile from '$lib/core/ui/Tile.svelte';
  import Chev from '$lib/core/ui/Chev.svelte';
  import StatePill from '$lib/core/ui/StatePill.svelte';
  import ChoiceRow from '$lib/core/ui/ChoiceRow.svelte';

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'chan_title', params);

  interface Props {
    detail: ChannelDetail;
    onclose: () => void;
    onforgotten?: () => void;
    /** The panel shows the moderation of the channel instead of the channel. */
    moderation?: boolean;
    modTab?: ModerationTab;
  }
  let { detail, onclose, onforgotten, moderation = $bindable(false), modTab = $bindable('queue') }: Props = $props();

  const cid = $derived(detail.channel.id);
  const me = $derived(myKey());
  const view = $derived(detail.channel);
  const queue = $derived(channelStore.queue[cid] ?? 0);
  const asks = $derived(channelStore.requests[cid] ?? 0);
  const levelAsks = $derived(channelStore.levelRequests[cid] ?? 0);

  /** What waits for me in the moderation, in words: «3 заявки · 2 на проверке». */
  const waiting = $derived.by(() => {
    const parts: string[] = [];
    if (asks) parts.push($t(countKey('chan_requests_count', asks, $locale), { n: String(asks) }));
    if (levelAsks) parts.push($t(countKey('chan_level_requests_count', levelAsks, $locale), { n: String(levelAsks) }));
    if (queue) parts.push($t(countKey('chan_queue_count', queue, $locale), { n: String(queue) }));
    return parts.join(' · ');
  });

  function openModeration() {
    modTab = asks ? 'requests' : levelAsks ? 'levels' : 'queue';
    moderation = true;
  }

  let error = $state('');
  let busy = $state(false);
  let editing = $state(false);
  let inviting = $state(false);
  let addingAuthor = $state(false);
  let name = $state('');
  let about = $state('');
  let join = $state('open');
  let history = $state('open');
  let menu = $state<{ open: boolean; x: number; y: number; s: StaffView | null }>({ open: false, x: 0, y: 0, s: null });
  /** The levels of one person, given or taken. */
  let levelsFor = $state<{ open: boolean; who: string }>({ open: false, who: '' });
  const lv = $derived(levelsOf(detail));
  /** The names of a person's levels besides the base one, as chips of their row. */
  const levelChips = (pubkey: string) => levelNames(lv.levels, levelsOfPerson(detail, pubkey));

  async function run(fn: () => Promise<unknown>): Promise<boolean> {
    error = ''; busy = true;
    try { await fn(); return true; }
    catch (e) { error = channelError(e, tr); return false; }
    finally { busy = false; }
  }

  const act = (a: ChannelAction) => run(() => channelStore.act(cid, a));

  async function sure(text: string, cta: string, a: ChannelAction): Promise<boolean> {
    return (await confirmStore.ask(text, cta, true)) && (await act(a));
  }

  const now = () => Math.floor(Date.now() / 1000);

  // ── The keeper ─────────────────────────────────────────────────────────

  let keeper = $state<KeeperStatus | null>(null);
  /** What the state of the keeper hangs on: read again when it changes or the keeper tells of itself, not at every message or read of the page. */
  const keeperSig = $derived(JSON.stringify([detail.keeper, view.kept_here, view.keeper_last_seen, view.membership, detail.has_key, detail.transfer ?? null]));
  $effect(() => {
    const id = cid;
    void channelStore.keeperRev[id];
    void keeperSig;
    untrack(() => channelsApi.keeper.status(id).then((k) => { if (id === cid) keeper = k; }).catch(() => { keeper = null; }));
  });

  // Where the files of the channel live: without Veydan Cloud on the free media of the
  // messenger, 14 days (communities-wire §9, the owner's rule of 2026-10-23); there a
  // history «с момента входа» is not offered, as its keys would go before a month.
  let plan = $state<{ free: boolean; retention_days: number }>({ free: false, retention_days: 0 });
  $effect(() => {
    channelsApi.storagePlan().then((p) => (plan = p)).catch(() => {});
    void serverStore.load();
  });
  const onOwnServer = $derived((serverStore.list ?? []).some((s) => !!s.media && view.storage.includes(s.media)));
  const freeMedia = $derived(plan.free && !onOwnServer);
  const histories = $derived(freeMedia && detail.history !== 'from_join' ? ['open'] : ['open', 'from_join']);

  /** The facts of the keeper kept here, for whoever wants them: closed by default. */
  let facts = $state(false);
  const phone = onPhone();

  /**
   * Whether the keeper is about, as every reader can tell: this device when
   * it keeps the channel, else the newest of its epochs and of its sign of
   * life (`keeper_last_seen`, wire §18), «online» when that is recent.
   */
  const keeperOnline = $derived(
    keeper?.kept_here ? keeper.online : !!view.keeper_last_seen && now() - view.keeper_last_seen < KEEPER_AWAY_SECS,
  );
  const keeperState = $derived.by(() => {
    if (keeperOnline) return $t('chan_keeper_online');
    if (!keeper?.kept_here && view.keeper_last_seen) return $t('chan_keeper_seen', { when: listStamp(view.keeper_last_seen) });
    return $t('chan_keeper_offline');
  });

  /** The keeper on a phone that runs it in the background: the owner's switch and its state. */
  const background = $derived(keeperBackground(keeper, phone));

  async function keepOn() {
    const on = !background.keepOn;
    await run(async () => { keeper = await channelsApi.keeper.keepOn(cid, on); });
  }

  /** Where the keeper is and what may be done about it (wire §15): taking it over here, its move, the recovery. */
  const move = $derived(keeperMove(keeper, !!detail.keeper, now()));

  /** «Сделать хранителем это устройство», or the recovery when the device that keeps it is lost. */
  async function take(emergency: boolean) {
    const text = tr(emergency ? 'chan_keeper_emergency_confirm' : 'chan_keeper_take_confirm');
    if (!(await confirmStore.ask(text, tr(emergency ? 'chan_keeper_emergency_cta' : 'chan_keeper_take_cta'), emergency))) return;
    await run(async () => { keeper = await channelsApi.keeper.take(cid, emergency); });
  }

  // ── A new owner (communities-wire §16) ──────────────────────────────────

  /** «Передать канал»: the journal names the new owner, he is offered the channel; nothing changes until he accepts. */
  async function transferTo(who: string) {
    if (!(await confirmStore.ask(tr('chan_transfer_confirm', { name: nameStore.label(who) }), tr('chan_transfer_cta'), true))) return;
    await run(async () => { await channelsApi.transfer.offer(cid, who); await channelStore.refresh(cid); });
  }

  /** My offer of the channel, while it waits or after it was declined. */
  const offered = $derived(myOffer(detail));

  async function rotateNow() {
    if (!(await confirmStore.ask(tr('chan_keeper_rotate_confirm'), tr('chan_keeper_rotate'), false))) return;
    await run(async () => { keeper = await channelsApi.keeper.rotateNow(cid); });
  }

  // ── The banned ─────────────────────────────────────────────────────────

  let banned = $state<string[]>([]);
  const unbans = $derived(canUnban(detail));
  // Read again with news of the journal (a ban, an unban), not at every read of the page.
  $effect(() => {
    const id = cid;
    void channelStore.journalRev[id];
    if (!unbans) { untrack(() => (banned = [])); return; }
    untrack(() => channelsApi.moderation.blocked(id).then((b) => { if (id === cid) banned = b; }).catch(() => {}));
  });

  /** The mute, the ban, and a change of role that leaves a moderator without it: asked and done there. */
  let acts = $state<PersonActions | null>(null);

  function untilText(at: number): string {
    const o = opensAt(at, now(), localeTag($locale));
    return o ? (o.day ? `${o.day} ${o.time}` : o.time) : stamp(at);
  }

  async function unban(who: string) {
    if (!(await act({ op: 'unban', who }))) return;
    banned = banned.filter((b) => b !== who);
    if (view.join === 'invite' && (await confirmStore.ask($t('chan_unban_invite', { name: nameStore.label(who) }), $t('chan_invite'), false))) {
      await run(() => channelStore.invite(cid, who));
    }
  }

  // ── Editing the card ───────────────────────────────────────────────────

  /** The step of history editing made, while it is the current one. */
  let stepped = false;

  function startEdit() {
    name = view.name; about = view.about; join = view.join; history = detail.history; editing = true;
    try { pushState('', { ...page.state, channelEdit: true } as App.PageState); stepped = true; } catch { stepped = false; }
  }

  function stopEdit() {
    if (stepped && (page.state as Record<string, unknown>).channelEdit) window.history.back();
    else { editing = false; stepped = false; }
  }

  // A back left the step of editing.
  $effect(() => {
    void (page.state as Record<string, unknown>).channelEdit;
    if (editing && stepped && !(page.state as Record<string, unknown>).channelEdit) { editing = false; stepped = false; }
  });

  function close() {
    if (editing) stopEdit();
    onclose();
  }

  async function save() {
    const a: Extract<ChannelAction, { op: 'settings' }> = { op: 'settings' };
    if (name.trim() && name.trim() !== view.name) a.name = name.trim();
    if (about !== view.about) a.about = about;
    if (join !== view.join) a.join = join;
    if (history !== detail.history) a.history = history;
    if (Object.keys(a).length === 1) { stopEdit(); return; }
    if (await act(a)) stopEdit();
  }

  /** A picture from the device as the cover: it takes effect at once, apart from «Сохранить». */
  async function pickCover() {
    let src: string | null = null;
    if (!isTauriHost) src = '/home/dev/Pictures/cover.png';
    else {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const picked = await open({ multiple: false, directory: false, filters: [{ name: $t('chan_cover'), extensions: ['jpg', 'jpeg', 'png'] }] });
      src = typeof picked === 'string' ? picked : null;
    }
    if (src) await run(() => channelStore.setAvatar(cid, src));
  }

  // ── The menu of a person ───────────────────────────────────────────────

  function openMenu(e: MouseEvent | { x: number; y: number }, s: StaffView) {
    if ('preventDefault' in e) e.preventDefault();
    if (!entriesFor(s).length) return;
    const x = 'clientX' in e ? e.clientX : e.x;
    const y = 'clientY' in e ? e.clientY : e.y;
    menu = { open: true, x, y, s };
  }

  function entriesFor(s: StaffView): MenuEntry[] {
    const who = nameStore.label(s.pubkey);
    const list: MenuEntry[] = [];
    const other = s.pubkey !== me;
    if (other) list.push({ label: $t('chan_member_write'), icon: 'send', onselect: () => { writeTo(s.pubkey).catch(() => {}); onclose(); } });
    if (other) list.push({ label: $t('chan_menu_report'), icon: 'alert-triangle', danger: true,
      // A person met in the channel: the channel is named, no post.
      onselect: () => reportStore.open({ accused: s.pubkey, name: who, context: { t: 'channel', community_id: detail.channel.id }, canBlock: false }) });
    for (const role of rolesFor(detail, s.pubkey, me)) {
      list.push({ label: $t(`chan_member_make_${role}` as 'chan_member_make_author'), icon: role === 'author' ? 'pencil' : 'shield',
        onselect: () => acts?.demoting(s.pubkey, role, () => act({ op: 'set_role', who: s.pubkey, role })) });
    }
    // The owner hands the channel to one of the team (communities-wire §16); one handover at a time.
    if (canTransferTo(detail, s.pubkey, me)) {
      list.push({ label: $t('chan_member_transfer'), icon: 'key', onselect: () => transferTo(s.pubkey) });
    }
    if (lv.levels.length > 1 && canSetLevels(detail, s.pubkey, me)) {
      list.push({ label: $t('chan_member_levels'), icon: 'layers', onselect: () => (levelsFor = { open: true, who: s.pubkey }) });
    }
    if (canMute(detail, s.pubkey, me)) {
      if (mutedNow(s)) list.push({ label: $t('chan_member_unmute'), icon: 'bell', onselect: () => run(() => channelsApi.moderation.mod(cid, { t: 'unmute', who: s.pubkey }).then(() => channelStore.refresh(cid))) });
      else list.push({ label: $t('chan_member_mute'), icon: 'bell-off', onselect: () => acts?.askMute(s.pubkey) });
    }
    if (canTrust(detail, s.pubkey, me) && checksMessages(detail)) {
      list.push(s.trusted
        ? { label: $t('chan_member_untrust'), icon: 'shield', onselect: () => run(() => channelsApi.moderation.mod(cid, { t: 'trust', who: s.pubkey, trusted: false }).then(() => channelStore.refresh(cid))) }
        : { label: $t('chan_member_trust'), icon: 'shield-check', onselect: () => run(() => channelsApi.moderation.mod(cid, { t: 'trust', who: s.pubkey, trusted: true }).then(() => channelStore.refresh(cid))) });
    }
    if (canRemove(detail, s.pubkey, me)) {
      list.push({ type: 'separator' });
      list.push({ label: $t('chan_member_remove'), icon: 'x', danger: true,
        onselect: () => acts?.demoting(s.pubkey, null, () => sure($t('chan_remove_confirm', { name: who }), $t('chan_member_remove'), { op: 'remove', who: s.pubkey })) });
    }
    if (canBan(detail, s.pubkey, me)) list.push({ label: $t('chan_member_ban'), icon: 'user-cancel', danger: true, onselect: () => acts?.askBan(s.pubkey) });
    return list;
  }

  const items = $derived(menu.s ? entriesFor(menu.s) : []);

  /** The team first by role, then those the journal mutes or trusts. */
  const ORDER: Role[] = ['owner', 'admin', 'moderator', 'author', 'member'];
  const team = $derived([...detail.staff].sort((a, b) => ORDER.indexOf(a.role as Role) - ORDER.indexOf(b.role as Role) || a.since - b.since));
  const days = $derived(weekdayNames(localeTag($locale)));
  const rules = $derived.by(() => { void $locale; return rulesLines(detail, tr, days); });

  async function leave() {
    if (await confirmStore.ask($t('chan_leave_confirm', { name: view.name }), $t('chan_leave'), true)) await act({ op: 'leave' });
  }

  async function disband() {
    if (await confirmStore.ask($t('chan_disband_confirm', { name: view.name }), $t('chan_disband'), true)) await act({ op: 'disband' });
  }

  async function forget() {
    if (!(await confirmStore.ask($t('chan_forget_confirm', { name: view.name }), $t('chan_forget'), true))) return;
    if (await run(() => channelStore.forget(cid))) onforgotten?.();
  }

  /** The channel itself: its owner answers for it, the context names it. */
  function reportChannel() {
    reportStore.open({ target: 'channel', accused: view.owner, name: view.name, context: { t: 'channel', community_id: cid }, canBlock: false });
  }

  async function cancelJoin() {
    if (!(await confirmStore.ask($t('chan_cancel_join_confirm', { name: view.name }), $t('chan_cancel_join'), true))) return;
    if (await run(() => channelStore.cancelJoin(cid))) onforgotten?.();
  }

  const fmt = (n: number) => new Intl.NumberFormat(localeTag($locale)).format(n);

  // ── The contents and its pages ─────────────────────────────────────────
  // The team of a joined channel sees the panel as contents: each row opens
  // the page of what it names, with a way back. Readers see it all at once.

  type Page = '' | 'link' | 'keeper' | 'levels' | 'sections' | 'team' | 'banned';
  let opened = $state<Page>('');
  const hub = $derived(joined(detail) && isModerator(detail));
  const pages = $derived<Record<Exclude<Page, ''>, boolean>>({
    link: hub && !!detail.link,
    keeper: hub,
    levels: hub && isManager(detail),
    sections: hub && isManager(detail),
    team: hub,
    banned: hub && banned.length > 0,
  });
  /** The page shown: none when what it showed is gone (the last ban lifted, a role taken). */
  const shown = $derived<Page>(opened && pages[opened] ? opened : '');
  const PAGE_TITLE: Record<Exclude<Page, ''>, string> = {
    link: 'chan_link', keeper: 'chan_keeper', levels: 'chan_levels', sections: 'chan_sections', team: 'chan_team', banned: 'chan_banned',
  };
  // Another channel starts from its contents.
  $effect(() => { void cid; untrack(() => (opened = '')); });
  // A page whose matter is gone is closed for good, not only hidden: it does not open again by itself when that comes back.
  $effect(() => { if (opened && !pages[opened]) opened = ''; });

  // The row pressed and the arrow back leave the page with the page: the focus goes on to the arrow of the
  // page opened, and back to the row it came from.
  let panelEl = $state<HTMLElement | null>(null);
  async function openPage(p: Exclude<Page, ''>) {
    opened = p;
    await tick();
    panelEl?.querySelector<HTMLElement>('header button')?.focus();
  }
  async function closePage() {
    const from = opened;
    opened = '';
    await tick();
    panelEl?.querySelector<HTMLElement>(`[data-page="${from}"]`)?.focus();
  }

  /** Where the keeper is, in words: the line of its card and of its row. */
  const moveText = $derived(
    move.place === 'here' ? $t('chan_keeper_here_line')
    : move.place === 'handing' ? $t('chan_keeper_handing')
    : move.place === 'taking' ? $t('chan_keeper_taking')
    : move.place === 'take' ? $t('chan_keeper_other_device')
    : move.place === 'elsewhere' ? $t('chan_keeper_elsewhere')
    : $t('chan_keeper_unknown'),
  );

  const joinWord = $derived(JOINS.includes(view.join) ? $t(`chan_join_${view.join}` as 'chan_join_open') : '');
  const levelList = $derived(orderedLevels(lv.levels).map((l) => l.name).join(', '));
  const scopeTitle = (s: { scope_id: string; title: string }) => (s.scope_id === EVERYONE && !s.title ? $t('chan_scope_everyone') : s.title);
  const scopeList = $derived(ordered(lv.scopes).map(scopeTitle).join(', '));

  /** A rule as the word of its setting; a value of a newer core as it came (as `choice` of lines.ts says it). */
  const word = (prefix: string, known: string[], v: string) => (known.includes(v) ? $t(`${prefix}${v}` as 'chan_join_open') : v);
  /**
   * The rules of the head as short facts, each with its icon: the join and the history each a fact of
   * its own (their words, not the first line of rulesLines cut apart), then the other lines of
   * rulesLines — the comments, the check when it is on, the hours when they are set, in that order.
   */
  const ruleFacts = $derived.by(() => {
    const icons = ['message-circle', ...(detail.premod !== 'off' ? ['shield'] : []), ...(detail.schedule?.days.length ? ['calendar'] : [])];
    return [
      { icon: 'user-question', text: word('chan_join_', JOINS, view.join) },
      { icon: 'clock', text: word('chan_history_', HISTORIES, detail.history) },
      ...rules.slice(1).map((text, i) => ({ icon: icons[i] ?? 'shield', text })),
    ];
  });

  const KINDS: [string, string][] = [['open', 'world'], ['request', 'user-question'], ['invite', 'lock']];
</script>

<!-- The keeper: its state, where it is, what may be done about it. -->
{#snippet keeperCard()}
  <Box tag="section" kind="text" tight aria-label={$t('chan_keeper')}>
    <div class="keeper-head">
      <Tile icon="key" tone="ok" />
      <span class="keeper-name">{$t('chan_keeper')}</span>
      <StatePill text={keeperState} tone={keeperOnline ? 'ok' : 'warn'} />
    </div>
    <p class="text" class:moving={move.place === 'handing' || move.place === 'taking'}>
      {#if move.place === 'handing' || move.place === 'taking'}<Icon name="loader" size={14} />{/if}
      {moveText}
    </p>
    {#if move.place === 'taking'}<p class="text">{$t('chan_keeper_taking_hint')}</p>{/if}
    {#if keeper?.kept_here && keeper.storage_error}<p class="text warn">{$t('chan_storage_refused', { why: keeper.storage_error })}</p>{/if}
    {#if keeper?.kept_here && freeMedia}<p class="text">{$t('chan_free_media_note', { days: String(plan.retention_days) })}</p>{/if}
    {#if move.place === 'take'}
      <button class="btn btn-ghost btn-sm wrap" disabled={busy} onclick={() => take(false)}><Icon name="smartphone" size={18} />{$t('chan_keeper_take')}</button>
    {/if}
    {#if move.emergency}
      <button class="btn btn-ghost btn-sm danger wrap" disabled={busy} onclick={() => take(true)}><Icon name="alert-triangle" size={18} />{$t('chan_keeper_emergency')}</button>
    {/if}
    {#if background.asleep}<p class="text warn">{$t('chan_keeper_phone')}</p>{/if}
    {#if background.switch}
      <div class="keep-on">
        <span class="keep-on-label">{$t('chan_keeper_keep_on')}</span>
        <button class="toggle" class:on={background.keepOn} role="switch" aria-checked={background.keepOn} aria-label={$t('chan_keeper_keep_on')}
          disabled={busy} onclick={keepOn}></button>
      </div>
      <p class="text dim">{$t('chan_keeper_keep_on_hint')}</p>
      {#if background.running}<p class="text running"><Icon name="check" size={14} />{$t('chan_keeper_background')}</p>{/if}
    {/if}
    {#if keeper?.kept_here && facts}
      <dl class="facts">
        {#if keeper.epoch !== undefined}<dt>{$t('chan_keeper_epoch')}</dt><dd>{keeper.epoch}{#if keeper.pending_epoch !== undefined} → {keeper.pending_epoch}{/if}</dd>{/if}
        {#if keeper.last_epoch_at}<dt>{$t('chan_keeper_last')}</dt><dd>{listStamp(keeper.last_epoch_at)}</dd>{/if}
        {#if keeper.next_wake}<dt>{$t('chan_keeper_next')}</dt><dd>{listStamp(keeper.next_wake)}</dd>{/if}
        <dt>{$t('chan_keeper_readers')}</dt><dd>{fmt(keeper.readers)}</dd>
        {#if keeper.generations > 1}<dt>{$t('chan_keeper_generations')}</dt><dd>{keeper.generations}</dd>{/if}
        {#if keeper.garbage}<dt>{$t('chan_keeper_garbage')}</dt><dd>{keeper.garbage}</dd>{/if}
      </dl>
    {/if}
    {#if keeper?.kept_here || canRotateKey(detail)}
      <div class="keeper-acts">
        {#if keeper?.kept_here}
          <button type="button" class="facts-toggle" aria-expanded={facts} onclick={() => (facts = !facts)}>
            {$t('chan_keeper_more')}<Icon name={facts ? 'chevron-up' : 'chevron-down'} size={14} />
          </button>
        {:else}<span></span>{/if}
        {#if canRotateKey(detail)}
          <button class="btn btn-ghost btn-sm" disabled={busy || !!keeper?.rotate_asked} onclick={rotateNow}>
            <Icon name="refresh-cw" size={18} />{keeper?.rotate_asked ? $t('chan_keeper_rotate_asked') : $t('chan_keeper_rotate')}</button>
        {/if}
      </div>
    {/if}
  </Box>
{/snippet}

<!-- The team: who may do what, the offer of the channel, the way to add. -->
{#snippet teamBody(heading: boolean)}
  <section class="block" aria-label={$t('chan_team')}>
    {#if heading}<div class="block-head"><span class="cap">{$t('chan_team')}<b class="count-pill quiet">{team.length}</b></span></div>{/if}
    {#if isManager(detail) || canInvite(detail)}
      <div class="two fit">
        {#if isManager(detail)}<button class="btn btn-ghost" onclick={() => (addingAuthor = true)}><Icon name="pencil" size={18} />{$t('chan_add_author')}</button>{/if}
        {#if canInvite(detail)}<button class="btn btn-ghost" onclick={() => (inviting = true)}><Icon name="user-plus" size={18} />{$t('chan_invite')}</button>{/if}
      </div>
    {/if}
    {#if offered}
      <p class="hint transfer" class:warn={offered.status === 'declined'}>
        <Icon name={offered.status === 'declined' ? 'x' : 'clock'} size={14} />
        {offered.status === 'declined' ? $t('chan_transfer_declined', { name: nameStore.label(offered.peer) }) : $t('chan_transfer_out', { name: nameStore.label(offered.peer) })}
      </p>
    {/if}
    {#if team.length}
      <Box tag="ul">
        {#each team as s (s.pubkey)}
          {@const nm = nameStore.label(s.pubkey)}
          <li>
            <button class="person as-button" class:still={!entriesFor(s).length} oncontextmenu={(e) => openMenu(e, s)} onclick={(e) => openMenu(e, s)}
              use:longpress={{ onpress: (p) => openMenu(p, s) }}>
              <Avatar url={nameStore.picture(s.pubkey)} label={nm} seed={s.pubkey} size={36} />
              <span class="who">
                <span class="who-name">{nm}{#if s.pubkey === me} <span class="me">({$t('chan_you')})</span>{/if}</span>
                <code>{shortKey(s.pubkey)}</code>
                {#if levelChips(s.pubkey).length}
                  <span class="lvls" title={$t('chan_levels')}>{#each levelChips(s.pubkey) as n (n)}<span class="lvl"><Icon name="layers" size={14} />{n}</span>{/each}</span>
                {/if}
              </span>
              {#if s.trusted && checksMessages(detail)}<span class="dim" title={$t('chan_member_trusted')}><Icon name="shield-check" size={16} /></span>{/if}
              {#if mutedNow(s)}
                <span class="dim" title={s.muted_until ? $t('chan_member_muted_until', { time: untilText(s.muted_until) }) : $t('chan_member_muted')}><Icon name="bell-off" size={16} /></span>
              {/if}
              {#if staffOf(detail, s.pubkey).role !== 'member'}<span class="role {s.role}">{$t(`chan_role_${s.role}` as 'chan_role_admin')}</span>{/if}
            </button>
          </li>
        {/each}
      </Box>
    {/if}
    {#if !isModerator(detail)}<p class="hint">{$t('chan_readers_hint')}</p>{/if}
  </section>

  <!-- The readers, for the team only (the owner's decision, 2026-10-25): the menu of a row is the team's. -->
  {#if joined(detail) && isModerator(detail)}
    <ChannelReaders {detail} onmenu={openMenu} hasMenu={(s) => entriesFor(s).length > 0} />
  {/if}
{/snippet}

{#snippet bannedBody(heading: boolean)}
  <section class="block" aria-label={$t('chan_banned')}>
    {#if heading}<div class="block-head"><span class="cap">{$t('chan_banned')}<b class="count-pill quiet">{banned.length}</b></span></div>{/if}
    <Box tag="ul">
      {#each banned as b (b)}
        <li class="person">
          <Avatar url={nameStore.picture(b)} label={nameStore.label(b)} seed={b} size={36} />
          <span class="who"><span class="who-name">{nameStore.label(b)}</span><code>{shortKey(b)}</code></span>
          <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => unban(b)}>{$t('chan_unban')}</button>
        </li>
      {/each}
    </Box>
  </section>
{/snippet}

<div class="panel" bind:this={panelEl}>
  {#if moderation && isModerator(detail)}
  <ModerationPanel {detail} bind:tab={modTab} onback={() => (moderation = false)} />
  {:else if shown}
  <PanelHead title={$t(PAGE_TITLE[shown] as 'chan_link')} onback={closePage} onclose={close} />
  <div class="scroll">
    {#if error}<div class="error-msg">{error}</div>{/if}
    {#if shown === 'link'}<ChannelLink {detail} heading={false} />
    {:else if shown === 'keeper'}{@render keeperCard()}
    {:else if shown === 'levels'}<LevelsEditor {detail} part="levels" />
    {:else if shown === 'sections'}<LevelsEditor {detail} part="sections" />
    {:else if shown === 'team'}{@render teamBody(false)}
    {:else if shown === 'banned'}{@render bannedBody(false)}
    {/if}
  </div>
  {:else}
  <PanelHead title={$t('chan_info')} onback={editing ? stopEdit : undefined} onclose={close} />

  <div class="scroll">
    {#if editing}
      <section class="edit">
        <div class="cover-edit">
          <Avatar {...cover(view.picture, cid)} label={view.name} seed={cid} size={72} />
          <div class="two fit">
            <button class="btn btn-ghost" disabled={busy} onclick={pickCover}><Icon name="camera" size={18} />{$t('chan_cover_set')}</button>
            {#if view.picture}<button class="btn btn-ghost danger" disabled={busy} onclick={() => run(() => channelStore.removeAvatar(cid))}><Icon name="trash-2" size={18} />{$t('chan_cover_remove')}</button>{/if}
          </div>
        </div>
        <input class="field" type="text" bind:value={name} maxlength="64" placeholder={$t('chan_name')} aria-label={$t('chan_name')} disabled={busy} />
        <textarea class="field" rows="3" bind:value={about} maxlength="500" placeholder={$t('chan_about')} aria-label={$t('chan_about')} disabled={busy}></textarea>
        <div class="choice">
          <span class="cap">{$t('chan_rule_join')}</span>
          <div class="kinds" role="radiogroup" aria-label={$t('chan_rule_join')}>
            {#each KINDS as [v, icon] (v)}
              <ChoiceRow surface="panel" {icon} title={$t(`chan_join_${v}` as 'chan_join_open')} hint={$t(`chan_join_${v}_hint` as 'chan_join_open_hint')}
                checked={join === v} disabled={busy} onclick={() => (join = v)} />
            {/each}
          </div>
        </div>
        <div class="choice">
          <span class="cap">{$t('chan_rule_history')}</span>
          <div class="chips" role="radiogroup" aria-label={$t('chan_rule_history')}>
            {#each histories as v (v)}
              <button type="button" class="chip" class:active={history === v} role="radio" aria-checked={history === v} disabled={busy} onclick={() => (history = v)}>{$t(`chan_history_${v}` as 'chan_history_open')}</button>
            {/each}
          </div>
          {#if histories.length === 1}<span class="hint">{$t('chan_history_needs_storage')}</span>{/if}
        </div>
      </section>
    {:else}
      <section class="hero">
        <div class="hero-row">
          {#if isManager(detail)}
            <button type="button" class="cover-btn" disabled={busy} onclick={pickCover} aria-label={$t('chan_cover_change')} title={$t('chan_cover_change')}>
              <Avatar {...cover(view.picture, cid)} label={view.name} seed={cid} size={60} />
              <span class="cam"><Icon name="camera" size={14} /></span>
            </button>
          {:else}
            <Avatar {...cover(view.picture, cid)} label={view.name} seed={cid} size={60} />
          {/if}
          <div class="hero-text">
            <div class="name">{view.name}</div>
            <div class="kind-line">
              <Icon name="speakerphone" size={14} />
              {$t('chan_kind')} · {$t('chan_readers_n', { n: fmt(view.members) })}
            </div>
            {#if view.kept_here}<span class="here" title={$t('chan_keeper_here_hint')}><Icon name="key" size={14} />{$t('chan_keeper_here')}</span>{/if}
          </div>
          {#if isManager(detail)}
            <button type="button" class="edit-btn" onclick={startEdit} aria-label={$t('chan_edit')} title={$t('chan_edit')}><Icon name="pencil" size={18} /></button>
          {/if}
        </div>
        {#if view.about}<div class="about"><MessageContent text={view.about} cards={false} /></div>{/if}
        <ul class="facts-list">{#each ruleFacts as f, i (i)}<li class="fact"><Icon name={f.icon} size={14} /><span>{f.text}</span></li>{/each}</ul>
      </section>

      {#if error}<div class="error-msg">{error}</div>{/if}

      {#if !joined(detail)}
        <div class="state"><Icon name="clock" size={16} />{view.membership === 'out' ? $t(outKey(view, 'chan_out_text') as 'chan_out_text') : $t(`chan_state_${view.membership}` as 'chan_state_out')}</div>
      {/if}

      {#if isModerator(detail)}
        <button class="mod-row" class:waits={!!waiting} onclick={openModeration}>
          <Tile icon="shield" tone={waiting ? 'accent' : 'plain'} />
          <span class="mod-text">
            <span class="mod-name">{$t('chan_mod_title')}</span>
            {#if waiting}<span class="mod-waiting">{waiting}</span>{:else}<span class="mod-hint">{$t('chan_mod_row_hint')}</span>{/if}
          </span>
          <Chev />
        </button>
      {/if}

      {#if hub}
        <!-- The contents: each row opens the page of what it names. -->
        {#if pages.link || pages.keeper}
          <Box fromText>
            {#if pages.link}
              <NavRow icon="link" page="link" title={$t('chan_link')} onclick={() => openPage('link')}>
                {#snippet sub()}<code class="key">{detail.link}</code>{/snippet}
                {#snippet end()}{#if joinWord}<span class="end">{joinWord}</span>{/if}{/snippet}
              </NavRow>
            {/if}
            <NavRow icon="key" page="keeper" title={$t('chan_keeper')} onclick={() => openPage('keeper')}>
              {#snippet sub()}{moveText}{/snippet}
              {#snippet end()}<StatePill text={keeperState} tone={keeperOnline ? 'ok' : 'warn'} />{/snippet}
            </NavRow>
          </Box>
        {/if}
        {#if pages.levels}
          <Box fromText>
            <NavRow icon="layers" page="levels" title={$t('chan_levels')} onclick={() => openPage('levels')} count={lv.levels.length}>
              {#snippet sub()}{levelList}{/snippet}
            </NavRow>
            <!-- `chan-sections`: the feed's «Настроить» brings an admin here. -->
            <NavRow id="chan-sections" page="sections" icon="folder" title={$t('chan_sections')} onclick={() => openPage('sections')} count={lv.scopes.length}>
              {#snippet sub()}{scopeList}{/snippet}
            </NavRow>
          </Box>
        {/if}
        <Box fromText>
          <NavRow icon="users" page="team" title={$t('chan_team')} onclick={() => openPage('team')} count={team.length}>
            {#snippet sub()}<span class="stack">{#each team.slice(0, 5) as s (s.pubkey)}<span class="stack-av"><Avatar url={nameStore.picture(s.pubkey)} label={nameStore.label(s.pubkey)} seed={s.pubkey} size={22} /></span>{/each}</span>{/snippet}
          </NavRow>
          {#if pages.banned}
            <NavRow icon="lock" page="banned" title={$t('chan_banned')} onclick={() => openPage('banned')} count={banned.length}>
              {#snippet sub()}{banned.map((b) => nameStore.label(b)).join(', ')}{/snippet}
            </NavRow>
          {/if}
        </Box>
      {:else}
        {#if joined(detail)}{@render keeperCard()}{/if}

        <!-- A reader's (and an author's) own levels; the team is in every section. -->
        {#if joined(detail) && !isModerator(detail) && (lv.levels.length > 1 || lv.scopes.length > 1)}<MyLevels {detail} />{/if}

        {#if joined(detail)}<ChannelLink {detail} />{/if}

        {#if joined(detail) && isManager(detail)}<LevelsEditor {detail} />{/if}

        {@render teamBody(true)}

        {#if banned.length}{@render bannedBody(true)}{/if}
      {/if}

      <section class="zone">
        {#if isOwner(detail)}<p class="hint">{$t('chan_owner_hint')}</p>{/if}
        <div class="zone-btns">
          {#if joined(detail) && detail.has_key && !isOwner(detail)}
            <button class="btn btn-ghost danger" disabled={busy} onclick={leave}><Icon name="x" size={18} />{$t('chan_leave')}</button>
          {/if}
          {#if isOwner(detail)}
            <button class="btn btn-ghost danger" disabled={busy} onclick={disband}><Icon name="trash-2" size={18} />{$t('chan_disband')}</button>
          {/if}
          {#if (view.membership === 'joining' || (joined(detail) && !detail.has_key)) && !isOwner(detail)}
            <button class="btn btn-ghost danger" disabled={busy} onclick={cancelJoin}><Icon name="x" size={18} />{$t('chan_cancel_join')}</button>
          {:else if !joined(detail)}
            <button class="btn btn-ghost danger" disabled={busy} onclick={forget}><Icon name="trash-2" size={18} />{$t('chan_forget')}</button>
          {/if}
          <!-- Anyone reports a channel, its owner and team too (Play, 2026-10-22; channelMenu.ts). -->
          <button class="btn btn-ghost danger" onclick={reportChannel}><Icon name="alert-triangle" size={18} />{$t('chan_report_channel')}</button>
        </div>
        <span class="since">{$t('chan_created', { date: stamp(view.created_at) })}</span>
      </section>
    {/if}
  </div>
  {#if editing}
    <!-- The verbs of the form stay at the foot of the panel, the form scrolls above them (the owner's rule of 2026-10-25). -->
    <footer class="foot">
      {#if error}<div class="error-msg">{error}</div>{/if}
      <div class="two">
        <button class="btn btn-ghost" disabled={busy} onclick={stopEdit}>{$t('chan_cancel')}</button>
        <button class="btn btn-primary" disabled={busy || !name.trim()} onclick={save}>{$t('chan_save')}</button>
      </div>
    </footer>
  {/if}
  {/if}
</div>

<InvitePicker bind:open={inviting} {detail} />
<InvitePicker bind:open={addingAuthor} {detail} purpose="author" />
<ContextMenu bind:open={menu.open} x={menu.x} y={menu.y} {items} onclose={() => (menu.open = false)} />
<PersonActions bind:this={acts} {detail} onerror={(e) => (error = e)} />
{#if levelsFor.who}<MemberLevelsDialog bind:open={levelsFor.open} {detail} who={levelsFor.who} />{/if}

<style>
  .panel { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--surface); }
  /* The ends stop the finger dead: Android's webview stretches an inner scroller (and the one it passes the rest to) past its end and back, which shook the panel at its bottom. */
  .scroll { flex: 1; min-height: 0; overflow-y: auto; overscroll-behavior: none; padding: var(--sp-4) var(--sp-4) var(--sp-6); display: flex; flex-direction: column; gap: var(--sp-4); }
  /* A tablet or a wide window: the column stays readable in the middle. */
  .scroll > :global(*), .foot > * { width: 100%; max-width: 560px; align-self: center; flex-shrink: 0; }
  .cap { display: flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-base); font-weight: var(--fw-bold); color: var(--text); }
  /* A block's total is the .count-pill of base.css, .quiet. */
  .hint { margin: 0; font-size: var(--fs-sm); color: var(--text-3); line-height: 1.45; }
  .two { display: grid; grid-template-columns: 1fr 1fr; gap: var(--sp-2); width: 100%; }
  .two .btn { min-height: var(--control-h); min-width: 0; gap: var(--sp-1); padding-inline: var(--sp-3); font-size: var(--fs-sm); line-height: 1.25; white-space: normal; text-wrap: balance; text-align: center; }
  .two .btn:only-child { grid-column: 1 / -1; }
  /* A pair with icons: side by side, sharing the width, while both fit on a line; else each on a row of its own. */
  .two.fit { display: flex; flex-wrap: wrap; }
  .two.fit .btn { flex: 1 1 auto; }
  .danger { color: var(--danger-text); }

  /* Who: the cover beside the name, the description, the rules as short facts two by two. */
  .hero { display: flex; flex-direction: column; gap: var(--sp-3); }
  .hero-row { display: flex; align-items: center; gap: var(--sp-3); }
  .hero-row > :global(.avatar) { flex: none; }
  .cover-edit :global(.avatar) { margin-bottom: var(--sp-1); }
  .cover-btn { position: relative; flex: none; display: inline-flex; padding: 0; border: none; background: none; border-radius: 50%; cursor: pointer; }
  /* Busy with something else: the cover stays as it is (not the dimming of base.css), only its camera fades. */
  .cover-btn:disabled { opacity: 1; cursor: default; }
  .cover-btn:disabled .cam { opacity: 0.45; }
  .cover-btn:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  .cam {
    position: absolute; right: -2px; bottom: -2px; display: grid; place-items: center; width: 26px; height: 26px; border-radius: 50%;
    background: var(--glass-bg-raised); border: 1px solid var(--border-2); color: var(--text);
  }
  .cover-btn:hover:not(:disabled) .cam { border-color: var(--accent-border); }
  .hero-text { flex: 1; min-width: 0; display: flex; flex-direction: column; align-items: flex-start; gap: var(--sp-1); }
  .name { font-size: var(--fs-md); font-weight: var(--fw-bold); line-height: 1.25; overflow-wrap: anywhere; word-break: break-word; }
  .kind-line { display: flex; align-items: center; flex-wrap: wrap; gap: var(--sp-1); font-size: var(--fs-sm); color: var(--text-3); }
  /* The pencil of the mockup (ci.js, heroC): a square of the surface, no border. */
  .edit-btn {
    align-self: flex-start; flex: none; display: grid; place-items: center; width: 36px; height: 36px; padding: 0; border: none; border-radius: var(--radius);
    background: var(--surface-2); color: var(--text); cursor: pointer; transition: background var(--dur-fast) var(--ease);
  }
  .edit-btn:hover { background: var(--surface-hover); }
  .edit-btn:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  .here {
    display: inline-flex; align-items: center; gap: var(--sp-1); height: 22px; padding: 0 var(--sp-2); border-radius: var(--radius-pill);
    background: var(--success-bg); color: var(--success-text, var(--success)); font-size: var(--fs-xs); font-weight: var(--fw-semibold);
  }
  .about { font-size: var(--fs-base); line-height: 1.5; color: var(--text); }
  .facts-list { list-style: none; margin: 0; padding: var(--sp-3) 0 0; border-top: 1px solid var(--border-2); display: grid; grid-template-columns: 1fr 1fr; gap: var(--sp-2) var(--sp-3); }
  .fact { display: flex; align-items: flex-start; gap: var(--sp-1); min-width: 0; font-size: var(--fs-xs); line-height: 1.35; color: var(--text-2); overflow-wrap: anywhere; }
  .fact :global(svg) { flex: none; margin-top: 1px; color: var(--text-3); }

  /* Editing the card; its verbs are the foot of the panel. */
  .edit { display: flex; flex-direction: column; gap: var(--sp-3); }
  .cover-edit { display: flex; flex-direction: column; align-items: center; gap: var(--sp-1); }
  .cover-edit .two { margin-top: var(--sp-2); }
  .field {
    width: 100%; font: inherit; font-size: var(--fs-base); color: var(--text); background: var(--surface-3);
    border: 1px solid var(--border); border-radius: var(--radius); padding: var(--sp-2) var(--sp-3); resize: vertical;
  }
  input.field { min-height: var(--control-h-lg); }
  .field:focus { outline: none; border-color: var(--accent-border); }
  .choice { display: flex; flex-direction: column; align-items: stretch; gap: var(--sp-2); text-align: left; }
  .kinds { display: flex; flex-direction: column; gap: var(--sp-2); }
  .chips { display: flex; flex-wrap: wrap; gap: var(--sp-2); }
  /* The bar of the form's verbs (STYLEGUIDE, «Unsaved form»): out of the scroll, always in sight. */
  .foot {
    flex: none; display: flex; flex-direction: column; gap: var(--sp-2); padding: var(--sp-3) var(--sp-4) calc(var(--sp-3) + var(--sab, 0px));
    border-top: 1px solid var(--border); background: var(--glass-bar); -webkit-backdrop-filter: var(--glass-filter); backdrop-filter: var(--glass-filter);
  }

  .state {
    display: flex; align-items: center; gap: var(--sp-2); padding: var(--sp-3) var(--sp-4); border-radius: var(--radius-md);
    background: var(--warn-bg); border: 1px solid var(--warn-border); color: var(--warn-text); font-size: var(--fs-base); font-weight: var(--fw-semibold); line-height: 1.45;
  }
  .state :global(svg) { flex: none; }

  /* The moderation: first of the contents. */
  .mod-row {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; padding: var(--sp-3); text-align: left; cursor: pointer; font: inherit; color: var(--text);
    border: 1px solid var(--border); border-radius: var(--radius-lg); background: var(--surface-2); transition: background var(--dur-fast) var(--ease);
  }
  .mod-row:hover { background: var(--surface-row-hover); }
  .mod-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .mod-name { font-size: var(--fs-body); font-weight: var(--fw-bold); }
  .mod-hint { font-size: var(--fs-xs); color: var(--text-3); line-height: 1.4; }
  .mod-row.waits { border-color: var(--accent-border); background: var(--accent-bg); }
  .mod-waiting { font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--accent-text); line-height: 1.4; }

  /* The contents: what stands in its rows. */
  .key { font-size: var(--fs-2xs); color: var(--text-3); }
  .end { flex: none; font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--text-2); }
  .stack { display: inline-flex; padding-left: var(--sp-2); margin-top: 2px; }
  .stack-av { display: inline-flex; margin-left: calc(-1 * var(--sp-2)); border-radius: 50%; box-shadow: 0 0 0 2px var(--surface-2); }

  /* The keeper. */
  .keeper-head { position: relative; display: flex; align-items: center; gap: var(--sp-3); }
  .keeper-name { flex: 1; min-width: 0; font-size: var(--fs-body); font-weight: var(--fw-bold); color: var(--text); }
  .text { margin: 0; font-size: var(--fs-sm); line-height: 1.5; color: var(--text-2); }
  .text.dim { color: var(--text-3); }
  .text.warn { color: var(--warn-text); }
  .text.running { display: flex; align-items: center; gap: var(--sp-1); color: var(--success-text, var(--success)); }
  .text.moving { display: flex; align-items: center; gap: var(--sp-1); }
  .facts { display: grid; grid-template-columns: auto 1fr; gap: 2px var(--sp-3); margin: 0; font-size: var(--fs-xs); }
  .facts dt { color: var(--text-3); }
  .facts dd { margin: 0; color: var(--text-body); font-variant-numeric: tabular-nums; }
  /* The key's button keeps its words on one line: beside the toggle when it fits, else on a row of its own at the right. */
  .keeper-acts .btn { min-width: 0; margin-left: auto; padding-inline: var(--sp-3); gap: var(--sp-1); white-space: normal; text-wrap: balance; text-align: left; }
  .keeper-acts .facts-toggle { flex: none; }
  .keeper-acts { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: var(--sp-2); margin-top: var(--sp-1); padding-top: var(--sp-3); border-top: 1px solid var(--border-2); }
  .keep-on { display: flex; align-items: center; gap: var(--sp-2); }
  .keep-on-label { flex: 1; font-size: var(--fs-base); color: var(--text); }
  /* A long word of the keeper's move wraps inside the panel instead of running out of it. */
  .btn.wrap {
    align-self: flex-start; white-space: normal; height: auto; min-height: var(--control-h-sm); padding-block: var(--sp-1);
    text-align: left; justify-content: flex-start; max-width: 100%;
  }
  .facts-toggle {
    display: inline-flex; align-items: center; gap: var(--sp-1); padding: 2px 0; border: none; background: none; cursor: pointer;
    font: inherit; font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--accent-text);
  }

  /* Blocks of a reader's panel and the pages: a title, a box of rows, a note. */
  .block { display: flex; flex-direction: column; gap: var(--sp-3); }
  .block-head { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); min-height: 24px; }
  .hint.transfer { display: flex; align-items: center; gap: var(--sp-1); }
  .hint.warn { color: var(--warn-text); }
  .person { display: flex; align-items: center; gap: var(--sp-3); min-height: 52px; padding: var(--sp-2) var(--sp-2) var(--sp-2) var(--sp-3); border-radius: var(--radius-md); min-width: 0; }
  .as-button { width: 100%; border: none; background: none; color: inherit; font: inherit; text-align: left; cursor: pointer; transition: background var(--dur-fast) var(--ease); }
  .as-button:hover { background: var(--surface-row-hover); }
  .as-button.still { cursor: default; }
  .as-button.still:hover { background: none; }
  .who { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
  .who-name { font-size: var(--fs-base); font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .who code { font-size: var(--fs-2xs); color: var(--text-3); }
  .me { color: var(--text-3); font-weight: var(--fw-medium); }
  .lvls { display: flex; flex-wrap: wrap; gap: var(--sp-1); margin-top: 2px; }
  .lvl {
    display: inline-flex; align-items: center; gap: var(--sp-1); max-width: 100%; height: 20px; padding: 0 var(--sp-2); border-radius: var(--radius-pill);
    font-size: var(--fs-2xs); font-weight: var(--fw-semibold); color: var(--accent-text); background: var(--accent-bg);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .dim { flex: none; color: var(--text-3); display: inline-flex; }
  .role {
    flex-shrink: 0; display: inline-flex; align-items: center; height: 22px; padding: 0 var(--sp-2); border-radius: var(--radius-pill);
    font-size: var(--fs-2xs); font-weight: var(--fw-bold); background: var(--surface-3); color: var(--text-2);
  }
  .role.owner { background: var(--accent-bg); color: var(--accent-text); }
  .role.admin { background: var(--success-bg); color: var(--success-text, var(--success)); }

  /* The last: what cannot be undone, apart. */
  .zone { display: flex; flex-direction: column; gap: var(--sp-3); margin-top: var(--sp-2); padding-top: var(--sp-4); border-top: 1px dashed var(--border-2); }
  .zone-btns { display: flex; flex-direction: column; gap: var(--sp-2); }
  .zone-btns .btn { min-height: var(--control-h); justify-content: flex-start; gap: var(--sp-2); padding: 0 var(--sp-4); }
  .since { font-size: var(--fs-xs); color: var(--text-3); text-align: center; }

  @media (pointer: coarse) {
    .person { min-height: 56px; }
    .field { font-size: var(--fs-md); }
    .two .btn, .zone-btns .btn { min-height: var(--control-h-lg); }
    /* The history's choices under a finger. */
    .chips .chip { min-height: var(--control-h); padding-inline: var(--sp-3); }
  }
</style>
