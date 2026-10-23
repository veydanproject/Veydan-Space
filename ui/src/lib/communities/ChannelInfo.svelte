<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->
<!-- Copied from ui/src/lib/messenger/groups/GroupInfo.svelte at 8ce09c39, then changed for channels. -->

<!--
  Everything about a channel except its feed: its card and rules, the
  keeper that hands out its keys, the link, the team (owner, admins,
  moderators, authors) and those the journal mutes or trusts, the banned,
  and the way out. Its moderation is a mode of the same panel. Readers are
  not listed: the journal does not know them (the keeper does).
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { untrack } from 'svelte';
  import { pushState } from '$app/navigation';
  import { page } from '$app/state';
  import { countKey, locale, localeTag, t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import ContextMenu, { type MenuEntry } from '$lib/core/ui/ContextMenu.svelte';
  import { Avatar, MessageContent, PanelHead, confirmStore, isTauriHost, listStamp, longpress, myKey, onPhone, reportStore, serverStore, stamp, writeTo } from '$lib/messenger/seams/shared';
  import ChannelLink from './ChannelLink.svelte';
  import InvitePicker from './InvitePicker.svelte';
  import { channelStore } from './channelStore.svelte';
  import { nameStore, shortKey } from './names.svelte';
  import { channelsApi, type ChannelAction, type ChannelDetail, type KeeperStatus, type StaffView } from './api';
  import { channelError } from './errors';
  import { keeperBackground, keeperMove } from './keeper';
  import { canTransferTo, myOffer } from './transfer';
  import { outKey, rulesLines } from './lines';
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
  import { levelNames, levelsOf, levelsOfPerson } from './levels/sections';

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
</script>

<div class="panel">
  {#if moderation && isModerator(detail)}
  <ModerationPanel {detail} bind:tab={modTab} onback={() => (moderation = false)} />
  {:else}
  <PanelHead title={$t('chan_info')} onback={editing ? stopEdit : undefined} onclose={close} />

  <div class="scroll">
    <section class="card-top">
      {#if editing}
        <div class="cover-edit">
          <Avatar {...cover(view.picture, cid)} label={view.name} seed={cid} size={64} />
          <div class="row">
            <button class="btn btn-ghost btn-sm" disabled={busy} onclick={pickCover}><Icon name="camera" size={13} />{$t('chan_cover_set')}</button>
            {#if view.picture}<button class="btn btn-ghost btn-sm danger" disabled={busy} onclick={() => run(() => channelStore.removeAvatar(cid))}>{$t('chan_cover_remove')}</button>{/if}
          </div>
        </div>
        <input class="field" type="text" bind:value={name} maxlength="64" placeholder={$t('chan_name')} aria-label={$t('chan_name')} disabled={busy} />
        <textarea class="field" rows="3" bind:value={about} maxlength="500" placeholder={$t('chan_about')} aria-label={$t('chan_about')} disabled={busy}></textarea>
        <div class="choice">
          <span class="choice-label">{$t('chan_rule_join')}</span>
          <div class="chips" role="radiogroup" aria-label={$t('chan_rule_join')}>
            {#each ['open', 'request', 'invite'] as v (v)}
              <button type="button" class="chip" class:active={join === v} role="radio" aria-checked={join === v} disabled={busy} onclick={() => (join = v)}>{$t(`chan_join_${v}` as 'chan_join_open')}</button>
            {/each}
          </div>
          <span class="hint">{$t(`chan_join_${join}_hint` as 'chan_join_open_hint')}</span>
        </div>
        <div class="choice">
          <span class="choice-label">{$t('chan_rule_history')}</span>
          <div class="chips" role="radiogroup" aria-label={$t('chan_rule_history')}>
            {#each histories as v (v)}
              <button type="button" class="chip" class:active={history === v} role="radio" aria-checked={history === v} disabled={busy} onclick={() => (history = v)}>{$t(`chan_history_${v}` as 'chan_history_open')}</button>
            {/each}
          </div>
          {#if histories.length === 1}<span class="hint">{$t('chan_history_needs_storage')}</span>{/if}
        </div>
        <div class="row">
          <button class="btn btn-primary btn-sm" disabled={busy || !name.trim()} onclick={save}>{$t('chan_save')}</button>
          <button class="btn btn-ghost btn-sm" disabled={busy} onclick={stopEdit}>{$t('chan_cancel')}</button>
        </div>
      {:else}
        <Avatar {...cover(view.picture, cid)} label={view.name} seed={cid} size={64} />
        <div class="name">{view.name}</div>
        <div class="kind">
          <Icon name="speakerphone" size={12} />
          {$t('chan_kind')} · {$t('chan_readers_n', { n: fmt(view.members) })}
          {#if view.kept_here}<span class="badge" title={$t('chan_keeper_here_hint')}><Icon name="key" size={11} />{$t('chan_keeper_here')}</span>{/if}
        </div>
        {#if view.about}<div class="about"><MessageContent text={view.about} cards={false} /></div>{/if}
        <ul class="rules">{#each rules as line (line)}<li>{line}</li>{/each}</ul>
        {#if isManager(detail)}
          <div class="row">
            <button class="btn btn-ghost btn-sm" disabled={busy} onclick={pickCover}><Icon name="camera" size={13} />{$t('chan_cover_change')}</button>
            <button class="btn btn-ghost btn-sm" onclick={startEdit}><Icon name="pencil" size={13} />{$t('chan_edit')}</button>
          </div>
        {/if}
      {/if}
    </section>

    {#if error}<div class="error-msg">{error}</div>{/if}

    {#if !joined(detail)}
      <div class="state">{view.membership === 'out' ? $t(outKey(view, 'chan_out_text') as 'chan_out_text') : $t(`chan_state_${view.membership}` as 'chan_state_out')}</div>
    {/if}

    {#if isModerator(detail)}
      <button class="mod-row" class:waits={!!waiting} onclick={openModeration}>
        <span class="mod-icon"><Icon name="shield" size={16} /></span>
        <span class="mod-text">
          <span class="mod-name">{$t('chan_mod_title')}</span>
          {#if waiting}<span class="mod-waiting">{waiting}</span>{:else}<span class="mod-hint">{$t('chan_mod_row_hint')}</span>{/if}
        </span>
        <Icon name="chevron-right" size={15} />
      </button>
    {/if}

    {#if joined(detail)}
      <section class="keeper" aria-label={$t('chan_keeper')}>
        <div class="keeper-head">
          <Icon name="key" size={14} />
          <span class="keeper-name">{$t('chan_keeper')}</span>
          <span class="keeper-state" class:on={keeperOnline}>{keeperState}</span>
        </div>
        <p class="hint" class:moving={move.place === 'handing' || move.place === 'taking'}>
          {#if move.place === 'handing' || move.place === 'taking'}<Icon name="loader" size={12} />{/if}
          {#if move.place === 'here'}{$t('chan_keeper_here_line')}{:else if move.place === 'handing'}{$t('chan_keeper_handing')}{:else if move.place === 'taking'}{$t('chan_keeper_taking')}{:else if move.place === 'take'}{$t('chan_keeper_other_device')}{:else if move.place === 'elsewhere'}{$t('chan_keeper_elsewhere')}{:else}{$t('chan_keeper_unknown')}{/if}
        </p>
        {#if move.place === 'taking'}<p class="hint">{$t('chan_keeper_taking_hint')}</p>{/if}
        {#if keeper?.kept_here && keeper.storage_error}<p class="hint warn">{$t('chan_storage_refused', { why: keeper.storage_error })}</p>{/if}
        {#if keeper?.kept_here && freeMedia}<p class="hint">{$t('chan_free_media_note', { days: String(plan.retention_days) })}</p>{/if}
        {#if move.place === 'take'}
          <button class="btn btn-ghost btn-sm wrap" disabled={busy} onclick={() => take(false)}><Icon name="smartphone" size={13} />{$t('chan_keeper_take')}</button>
        {/if}
        {#if move.emergency}
          <button class="btn btn-ghost btn-sm danger wrap" disabled={busy} onclick={() => take(true)}><Icon name="alert-triangle" size={13} />{$t('chan_keeper_emergency')}</button>
        {/if}
        {#if background.asleep}<p class="hint warn">{$t('chan_keeper_phone')}</p>{/if}
        {#if background.switch}
          <div class="keep-on">
            <span class="keep-on-label">{$t('chan_keeper_keep_on')}</span>
            <button class="toggle" class:on={background.keepOn} role="switch" aria-checked={background.keepOn} aria-label={$t('chan_keeper_keep_on')}
              disabled={busy} onclick={keepOn}></button>
          </div>
          <p class="hint">{$t('chan_keeper_keep_on_hint')}</p>
          {#if background.running}<p class="hint running"><Icon name="check" size={12} />{$t('chan_keeper_background')}</p>{/if}
        {/if}
        {#if keeper?.kept_here}
          <button type="button" class="facts-toggle" aria-expanded={facts} onclick={() => (facts = !facts)}>
            {$t('chan_keeper_more')}<Icon name={facts ? 'chevron-up' : 'chevron-down'} size={13} />
          </button>
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
        {#if canRotateKey(detail)}
          <button class="btn btn-ghost btn-sm" disabled={busy || !!keeper?.rotate_asked} onclick={rotateNow}>
            <Icon name="refresh-cw" size={13} />{keeper?.rotate_asked ? $t('chan_keeper_rotate_asked') : $t('chan_keeper_rotate')}</button>
        {/if}
      </section>
    {/if}

    <!-- A reader's (and an author's) own levels; the team is in every section. -->
    {#if joined(detail) && !isModerator(detail) && (lv.levels.length > 1 || lv.scopes.length > 1)}<MyLevels {detail} />{/if}

    {#if joined(detail)}<ChannelLink {detail} />{/if}

    {#if joined(detail) && isManager(detail)}<LevelsEditor {detail} />{/if}

    <section class="block">
      <div class="label-row">
        <span class="label">{$t('chan_team')} · {team.length}</span>
        <span class="acts">
          {#if isManager(detail)}<button class="btn btn-ghost btn-sm" onclick={() => (addingAuthor = true)}><Icon name="pencil" size={13} />{$t('chan_add_author')}</button>{/if}
          {#if canInvite(detail)}<button class="btn btn-ghost btn-sm" onclick={() => (inviting = true)}><Icon name="user-plus" size={13} />{$t('chan_invite')}</button>{/if}
        </span>
      </div>
      {#if offered}
        <p class="hint transfer" class:warn={offered.status === 'declined'}>
          <Icon name={offered.status === 'declined' ? 'x' : 'clock'} size={12} />
          {offered.status === 'declined' ? $t('chan_transfer_declined', { name: nameStore.label(offered.peer) }) : $t('chan_transfer_out', { name: nameStore.label(offered.peer) })}
        </p>
      {/if}
      <ul class="list divided">
        {#each team as s (s.pubkey)}
          {@const nm = nameStore.label(s.pubkey)}
          <li>
            <button class="person as-button" class:still={!entriesFor(s).length} oncontextmenu={(e) => openMenu(e, s)} onclick={(e) => openMenu(e, s)}
              use:longpress={{ onpress: (p) => openMenu(p, s) }}>
              <Avatar url={nameStore.picture(s.pubkey)} label={nm} seed={s.pubkey} size={34} />
              <span class="who">
                <span class="who-name">{nm}{#if s.pubkey === me} <span class="me">({$t('chan_you')})</span>{/if}</span>
                <code>{shortKey(s.pubkey)}</code>
                {#if levelChips(s.pubkey).length}
                  <span class="lvls" title={$t('chan_levels')}>{#each levelChips(s.pubkey) as n (n)}<span class="lvl"><Icon name="layers" size={10} />{n}</span>{/each}</span>
                {/if}
              </span>
              {#if s.trusted && checksMessages(detail)}<span class="dim" title={$t('chan_member_trusted')}><Icon name="shield-check" size={13} /></span>{/if}
              {#if mutedNow(s)}
                <span class="dim" title={s.muted_until ? $t('chan_member_muted_until', { time: untilText(s.muted_until) }) : $t('chan_member_muted')}><Icon name="bell-off" size={13} /></span>
              {/if}
              {#if staffOf(detail, s.pubkey).role !== 'member'}<span class="role {s.role}">{$t(`chan_role_${s.role}` as 'chan_role_admin')}</span>{/if}
            </button>
          </li>
        {/each}
      </ul>
      <p class="hint">{$t('chan_readers_hint')}</p>
    </section>

    {#if banned.length}
      <section class="block">
        <div class="label">{$t('chan_banned')} · {banned.length}</div>
        <ul class="list divided">
          {#each banned as b (b)}
            <li class="person">
              <Avatar url={nameStore.picture(b)} label={nameStore.label(b)} seed={b} size={34} />
              <span class="who"><span class="who-name">{nameStore.label(b)}</span><code>{shortKey(b)}</code></span>
              <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => unban(b)}>{$t('chan_unban')}</button>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    <section class="block out">
      {#if joined(detail) && detail.has_key && !isOwner(detail)}
        <button class="btn btn-ghost btn-sm danger" disabled={busy} onclick={leave}><Icon name="x" size={13} />{$t('chan_leave')}</button>
      {/if}
      {#if isOwner(detail)}
        <p class="hint">{$t('chan_owner_hint')}</p>
        <button class="btn btn-ghost btn-sm danger" disabled={busy} onclick={disband}><Icon name="trash-2" size={13} />{$t('chan_disband')}</button>
      {/if}
      {#if (view.membership === 'joining' || (joined(detail) && !detail.has_key)) && !isOwner(detail)}
        <button class="btn btn-ghost btn-sm danger" disabled={busy} onclick={cancelJoin}><Icon name="x" size={13} />{$t('chan_cancel_join')}</button>
      {:else if !joined(detail)}
        <button class="btn btn-ghost btn-sm danger" disabled={busy} onclick={forget}><Icon name="trash-2" size={13} />{$t('chan_forget')}</button>
      {/if}
      <!-- Anyone reports a channel, its owner and team too (Play, 2026-10-22; channelMenu.ts). -->
      <button class="btn btn-ghost btn-sm danger" onclick={reportChannel}><Icon name="alert-triangle" size={13} />{$t('chan_report_channel')}</button>
      <span class="since">{$t('chan_created', { date: stamp(view.created_at) })}</span>
    </section>
  </div>
  {/if}
</div>

<InvitePicker bind:open={inviting} {detail} />
<InvitePicker bind:open={addingAuthor} {detail} purpose="author" />
<ContextMenu bind:open={menu.open} x={menu.x} y={menu.y} {items} onclose={() => (menu.open = false)} />
<PersonActions bind:this={acts} {detail} onerror={(e) => (error = e)} />
{#if levelsFor.who}<MemberLevelsDialog bind:open={levelsFor.open} {detail} who={levelsFor.who} />{/if}

<style>
  .panel { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--surface); }
  .scroll { flex: 1; min-height: 0; overflow-y: auto; padding: var(--sp-4); display: flex; flex-direction: column; gap: var(--sp-5); }
  .card-top { display: flex; flex-direction: column; align-items: center; gap: var(--sp-2); text-align: center; }
  .cover-edit { display: flex; flex-direction: column; align-items: center; gap: var(--sp-2); }
  .name { font-size: var(--fs-md); font-weight: var(--fw-extrabold); letter-spacing: -0.2px; overflow-wrap: anywhere; word-break: break-word; }
  .kind { display: inline-flex; align-items: center; flex-wrap: wrap; justify-content: center; gap: 5px; font-size: var(--fs-xs); color: var(--text-3); }
  .badge { display: inline-flex; align-items: center; gap: 3px; padding: 1px 7px; border-radius: var(--radius-pill); background: var(--success-bg); color: var(--success-text, var(--success)); font-weight: var(--fw-semibold); }
  .about { color: var(--text-body); }
  .rules { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; font-size: var(--fs-xs); color: var(--text-2); }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.45; }
  .field {
    width: 100%; font: inherit; font-size: var(--fs-sm); color: var(--text); background: var(--surface-2);
    border: 1px solid var(--border); border-radius: var(--radius-field); padding: 8px 10px; resize: vertical;
  }
  .field:focus { outline: none; border-color: var(--accent-border); }
  .choice { display: flex; flex-direction: column; align-items: flex-start; gap: 6px; width: 100%; text-align: left; }
  .choice-label { font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--text-2); }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  @media (pointer: coarse) { .chip { min-height: 40px; padding-inline: 14px; } }
  .row { display: flex; gap: 6px; flex-wrap: wrap; justify-content: center; }
  .state { padding: var(--sp-2) var(--sp-3); border-radius: var(--radius-md); background: var(--warn-bg); border: 1px solid var(--warn-border); color: var(--warn-text); font-size: var(--fs-xs); line-height: 1.45; }
  .block { display: flex; flex-direction: column; gap: var(--sp-2); }
  .block.out { align-items: flex-start; }
  .label { font-size: var(--fs-2xs); text-transform: uppercase; letter-spacing: 0.6px; color: var(--text-3); font-weight: var(--fw-bold); }
  .label-row { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); flex-wrap: wrap; }
  .acts { display: inline-flex; gap: 2px; flex-wrap: wrap; }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; --rows-gap: 2px; }
  .person { display: flex; align-items: center; gap: var(--sp-3); padding: 6px 8px; border-radius: var(--radius-sm); min-width: 0; }
  .as-button { width: 100%; border: none; background: none; color: inherit; font: inherit; text-align: left; cursor: pointer; }
  .as-button:hover { background: var(--surface-row-hover); }
  .as-button.still { cursor: default; }
  .as-button.still:hover { background: none; }
  .who { display: flex; flex-direction: column; gap: 1px; min-width: 0; flex: 1; }
  .who-name { font-size: var(--fs-sm); font-weight: var(--fw-semibold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .who code { font-family: var(--font-mono); font-size: var(--fs-2xs); color: var(--text-3); }
  .me { color: var(--text-3); font-weight: var(--fw-normal, 400); }
  .lvls { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 2px; }
  .lvl {
    display: inline-flex; align-items: center; gap: 3px; max-width: 100%; padding: 0 7px; border-radius: var(--radius-pill);
    font-size: var(--fs-2xs); font-weight: var(--fw-semibold); color: var(--accent-text-2); background: var(--accent-tint);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .dim { color: var(--text-3); display: inline-flex; }
  /* As on a post: an edge, not a grey fill the dark theme loses. */
  .role { flex-shrink: 0; font-size: var(--fs-2xs); font-weight: var(--fw-bold); padding: 1px 7px; border-radius: var(--radius-pill); border: 1px solid var(--border-strong); background: transparent; color: var(--text-2); }
  .role.owner { border-color: transparent; background: var(--accent-tint); color: var(--accent-text-2); }
  .role.admin { border-color: transparent; background: var(--success-bg); color: var(--success-text, var(--success)); }
  .keeper { display: flex; flex-direction: column; gap: 6px; padding: var(--sp-3); border-radius: var(--radius-md); border: 1px solid var(--border); background: var(--surface-2); }
  .keeper-head { display: flex; align-items: center; gap: 6px; color: var(--text-2); }
  .keeper-name { flex: 1; font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text); }
  .keeper-state { font-size: var(--fs-2xs); font-weight: var(--fw-bold); padding: 2px 8px; border-radius: var(--radius-pill); background: var(--warn-bg); color: var(--warn-text); }
  .keeper-state.on { background: var(--success-bg); color: var(--success-text, var(--success)); }
  .facts { display: grid; grid-template-columns: auto 1fr; gap: 2px var(--sp-3); margin: 0; font-size: var(--fs-xs); }
  .facts dt { color: var(--text-3); }
  .facts dd { margin: 0; color: var(--text-body); font-variant-numeric: tabular-nums; }
  .keeper .btn { align-self: flex-start; }
  .keep-on { display: flex; align-items: center; gap: var(--sp-2); }
  .keep-on-label { flex: 1; font-size: var(--fs-sm); color: var(--text); }
  .hint.running { display: flex; align-items: center; gap: 4px; color: var(--success-text, var(--success)); }
  .hint.warn { color: var(--warn-text); }
  .hint.moving { display: flex; align-items: center; gap: 4px; color: var(--text-2); }
  .hint.transfer { display: flex; align-items: center; gap: 4px; padding: 0 2px 6px; }
  /* A long word of the keeper's move wraps inside the panel instead of running out of it. */
  .btn.wrap { white-space: normal; height: auto; min-height: 30px; padding-top: 5px; padding-bottom: 5px; text-align: left; justify-content: flex-start; max-width: 100%; }
  .facts-toggle {
    align-self: flex-start; display: inline-flex; align-items: center; gap: 4px; padding: 2px 0; border: none; background: none; cursor: pointer;
    font: inherit; font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--accent-text-2);
  }
  .danger { color: var(--danger-text); }
  .since { font-size: var(--fs-2xs); color: var(--text-3); }
  .mod-row {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; padding: 10px 12px; text-align: left; cursor: pointer; font: inherit; color: var(--text);
    border: 1px solid var(--border); border-radius: var(--radius-md); background: var(--surface-2);
  }
  .mod-row:hover { background: var(--surface-row-hover); }
  .mod-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .mod-name { font-size: var(--fs-sm); font-weight: var(--fw-semibold); }
  .mod-hint { font-size: var(--fs-2xs); color: var(--text-3); line-height: 1.35; }
  .mod-icon {
    width: 32px; height: 32px; flex-shrink: 0; display: inline-flex; align-items: center; justify-content: center;
    border-radius: var(--radius-sm); background: var(--surface-3); color: var(--text-2);
  }
  .mod-row.waits { border-color: var(--accent-border); }
  .mod-row.waits .mod-icon { background: var(--accent-tint); color: var(--accent-text); }
  .mod-waiting { font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--accent-text); line-height: 1.35; }
  @media (pointer: coarse) {
    .person { padding: 9px 8px; }
    .field { font-size: var(--fs-md); }
  }
</style>
