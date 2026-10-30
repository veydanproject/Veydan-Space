<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Everything about a group except its conversation: who is in, in which
  role, who asks to be let in, the link, the settings, and the way out.
  Its moderation is a mode of the same panel, as a section of what the
  group shared is.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { tick, untrack } from 'svelte';
  import { pushState } from '$app/navigation';
  import { page } from '$app/state';
  import { countKey, locale, localeTag, t, type TranslationKey } from '$lib/core/i18n';
  import { membersKey } from '../i18n';
  import { groupError, moderationError } from './errors';
  import Icon from '$lib/core/Icon.svelte';
  import PanelHead from '../shared/PanelHead.svelte';
  import ContextMenu, { type MenuEntry } from '$lib/core/ui/ContextMenu.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import AvatarPicker from '../contacts/AvatarPicker.svelte';
  import GroupLink from './GroupLink.svelte';
  import MessageContent from '../content/MessageContent.svelte';
  import InvitePicker from './InvitePicker.svelte';
  import SharedMedia from '../content/shared/SharedMedia.svelte';
  import SharedList from '../content/shared/SharedList.svelte';
  import NavRow from '$lib/core/ui/NavRow.svelte';
  import Tile, { type TileTone } from '$lib/core/ui/Tile.svelte';
  import Chev from '$lib/core/ui/Chev.svelte';
  import StatePill, { type PillTone } from '$lib/core/ui/StatePill.svelte';
  import { groupStore } from './groupStore.svelte';
  import { nameStore, shortKey } from './names.svelte';
  import { canMute, canRemove, canTrust, checksMessages, hasModeration, isManager, isModerator, isOwner, losesModeration, mutedNow, rolesFor, seesModeration } from './permissions';
  import ModerationPanel, { type ModerationTab } from './moderation/ModerationPanel.svelte';
  import ChoiceDialog from './moderation/ChoiceDialog.svelte';
  import { MUTE_SPANS, muteUntil, opensAt, type MuteSpan } from './moderation/schedule';
  import { chatStore } from '../chats/chatStore.svelte';
  import { confirmStore } from '../shared/confirm.svelte';
  import { reportStore } from '../shared/report.svelte';
  import { longpress } from '../shared/longpress';
  import { stamp } from '../shared/time';
  import { messengerApi, type GroupAction, type GroupRole, type MessengerGroup, type MessengerGroupKey, type MessengerGroupMember, type ModAction, type SharedSection } from '../api';

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as "msg_you", params);

  interface Props {
    group: MessengerGroup;
    onclose: () => void;
    onforgotten?: () => void;
    /** The panel shows the moderation of the group instead of the group. */
    moderation?: boolean;
    /** The tab of the moderation shown. */
    modTab?: ModerationTab;
  }
  let { group, onclose, onforgotten, moderation = $bindable(false), modTab = $bindable('queue') }: Props = $props();

  /** What waits for me in the moderation, in words: «3 заявки · 2 на проверке». */
  const waiting = $derived.by(() => {
    const parts: string[] = [];
    const asks = isManager(group) ? group.requests.length : 0;
    const queue = hasModeration(group) ? (group.queue ?? 0) : 0;
    if (asks) parts.push($t(countKey('msg_greq_count', asks, $locale), { n: String(asks) }));
    if (queue) parts.push($t(countKey('msg_mod_queue_count', queue, $locale), { n: String(queue) }));
    return parts.join(' · ');
  });

  /** The moderation opens where something waits: on the requests when there are any. */
  function openModeration() {
    modTab = isManager(group) && group.requests.length ? 'requests' : hasModeration(group) ? 'queue' : 'requests';
    moderation = true;
  }

  let error = $state('');
  let busy = $state(false);
  let editing = $state(false);
  let inviting = $state(false);
  let name = $state('');
  let about = $state('');
  let history = $state(true);
  /** A section of what the group has shared, shown in place of the panel. */
  let section = $state<SharedSection | null>(null);
  let menu = $state<{ open: boolean; x: number; y: number; m: MessengerGroupMember | null }>({ open: false, x: 0, y: 0, m: null });

  const joined = $derived(group.membership === 'joined');
  const manager = $derived(isManager(group));

  function explain(e: unknown): string {
    return groupError(e, tr);
  }

  async function run(fn: () => Promise<unknown>) {
    error = ''; busy = true;
    try { await fn(); return true; }
    catch (e) { error = explain(e); return false; }
    finally { busy = false; }
  }

  const act = (a: GroupAction) => run(() => groupStore.act(group.id, a));

  async function sure(text: string, cta: string, a: GroupAction): Promise<boolean> {
    return (await confirmStore.ask(text, cta, true)) && (await act(a));
  }

  const now = () => Math.floor(Date.now() / 1000);

  /** A decision of a moderator; it changes the members, so the group is read again. */
  async function moderate(a: ModAction): Promise<boolean> {
    error = ''; busy = true;
    try {
      await messengerApi.moderation.groupMod(group.id, a);
      await groupStore.refresh(group.id);
      return true;
    } catch (e) {
      error = moderationError(e, tr);
      return false;
    } finally {
      busy = false;
    }
  }

  // ── A moderator who stops being one ────────────────────────────────────
  // Their decisions hold while they do (contract D: a mark is in force by
  // the log of now); the one who demotes, removes or bans them is offered
  // to sign those decisions again as their own.

  // What the inheritance signs again: the marks still in force (a hiding, an
  // approval); a rejection or an unhiding leaves nothing to carry.
  const MARKS = new Set(['hide', 'approve']);

  /** How many decisions on messages the log credits to this person; `null` when it cannot be read. */
  async function decisionsOf(who: string): Promise<number | null> {
    try {
      let n = 0;
      let before: number | undefined;
      for (let i = 0; i < 10; i++) {
        const page = await messengerApi.moderation.groupLog(group.id, before, 100);
        n += page.items.filter((e) => e.by === who && MARKS.has(e.action)).length;
        if (!page.next_before) break;
        before = page.next_before;
      }
      return n;
    } catch {
      return null;
    }
  }

  async function offerInherit(who: string, n: number | null) {
    if (n === 0) return;
    const name = nameStore.label(who);
    const text = n === null ? $t('msg_mod_inherit_confirm_plain', { name }) : $t('msg_mod_inherit_confirm', { name, n: String(n) });
    if (!(await confirmStore.ask(text, $t('msg_mod_inherit_cta'), false))) return;
    error = '';
    try { await messengerApi.moderation.groupInherit(group.id, who); }
    catch (e) { error = moderationError(e, tr); }
  }

  /** An operation that may leave a moderator below one: their decisions are counted before it, offered after (where the core keeps marks). */
  async function demoting(m: MessengerGroupMember, after: GroupRole | null, op: () => Promise<boolean>) {
    const losing = hasModeration(group) && losesModeration(m.role, after);
    const n = losing ? await decisionsOf(m.pubkey) : 0;
    if ((await op()) && losing) await offerInherit(m.pubkey, n);
  }

  // ── Choices a menu entry cannot hold ───────────────────────────────────

  let mute = $state<{ open: boolean; m: MessengerGroupMember | null; span: string }>({ open: false, m: null, span: 'hour' });
  let ban = $state<{ open: boolean; m: MessengerGroupMember | null; hide: boolean }>({ open: false, m: null, hide: true });
  let dialogBusy = $state(false);
  let dialogError = $state('');

  const muteOptions = $derived(MUTE_SPANS.map((v) => ({ value: v, label: $t(`msg_mod_mute_span_${v}` as 'msg_mod_mute_span_hour') })));

  async function confirmMute() {
    const m = mute.m;
    if (!m) return;
    dialogBusy = true; dialogError = '';
    try {
      const until = muteUntil(mute.span as MuteSpan, now());
      await messengerApi.moderation.groupMod(group.id, until === undefined ? { t: 'mute', who: m.pubkey } : { t: 'mute', who: m.pubkey, until });
      mute.open = false;
      await groupStore.refresh(group.id);
    } catch (e) {
      dialogError = moderationError(e, tr);
    } finally {
      dialogBusy = false;
    }
  }

  /** A ban, and with it, unless unticked, the hiding of everything they wrote until now. */
  async function confirmBan() {
    const m = ban.m;
    if (!m) return;
    const hide = ban.hide;
    await demoting(m, null, async () => {
      dialogBusy = true; dialogError = '';
      try {
        await groupStore.act(group.id, { op: 'ban', who: m.pubkey });
      } catch (e) {
        dialogError = explain(e);
        return false;
      } finally {
        dialogBusy = false;
      }
      ban.open = false;
      if (hide) await moderate({ t: 'hide_author', author: m.pubkey, from: 0, to: now() });
      return true;
    });
  }

  /** `до 18:00`, `до пт 18:00`: the end of a mute for a time, on this device's clock. */
  function untilText(at: number): string {
    const o = opensAt(at, now(), localeTag($locale));
    return o ? (o.day ? `${o.day} ${o.time}` : o.time) : stamp(at);
  }

  /** Unbanning does not bring anyone back: offer to invite them. */
  async function unban(who: string) {
    if (!(await act({ op: 'unban', who }))) return;
    if (await confirmStore.ask($t('msg_group_unban_invite', { name: nameStore.label(who) }), $t('msg_group_invite'), false)) {
      await run(() => groupStore.invite(group.id, who));
    }
  }

  function startEdit() {
    name = group.name; about = group.about; history = group.history_for_new; editing = true;
    try { pushState('', { ...page.state, groupEdit: true }); stepped = true; } catch { stepped = false; }
  }

  // Editing is a step of history, as the media viewer is: the way back (the
  // arrow of the panel, the phone's gesture and button) leaves the editing
  // for the group's info, not the info for the chat. The picture is applied
  // at once, so what was set stays set.
  /** The step of history editing made, while it is the current one. */
  let stepped = false;

  /** Back from editing to the info: over the step it made, which ends it. */
  function stopEdit() {
    if (stepped && page.state.groupEdit) window.history.back();
    else { editing = false; stepped = false; }
  }

  // A back left the step of editing.
  $effect(() => {
    void page.state.groupEdit;
    if (editing && stepped && !page.state.groupEdit) { editing = false; stepped = false; }
  });

  /** The panel closes: an editing under way leaves its step first. */
  function close() {
    if (editing) stopEdit();
    onclose();
  }

  async function save() {
    const a: GroupAction = { op: 'edit_settings' };
    if (name.trim() && name.trim() !== group.name) a.name = name.trim();
    if (about !== group.about) a.about = about;
    if (group.kind === 'private' && history !== group.history_for_new) a.history_for_new = history;
    if (Object.keys(a).length === 1) { stopEdit(); return; }
    if (await act(a)) { stopEdit(); chatStore.scheduleChatsRefresh(); }
  }

  function openMenu(e: MouseEvent | { x: number; y: number }, m: MessengerGroupMember) {
    if ('preventDefault' in e) e.preventDefault();
    if (!entriesFor(m).length) return;
    const x = 'clientX' in e ? e.clientX : e.x;
    const y = 'clientY' in e ? e.clientY : e.y;
    menu = { open: true, x, y, m };
  }

  function entriesFor(m: MessengerGroupMember): MenuEntry[] {
    const who = nameStore.label(m.pubkey);
    const list: MenuEntry[] = [];
    if (!m.is_me) list.push({ label: $t('msg_group_member_write'), icon: 'message-circle', onselect: () => { chatStore.openPeer(m.pubkey).catch(() => {}); onclose(); } });
    // Any member may report another, whatever either one's role.
    if (!m.is_me) list.push({ label: $t('msg_report'), icon: 'alert-triangle', danger: true,
      onselect: () => reportStore.open({ accused: m.pubkey, name: who, context: { t: 'group', group_id: group.id }, canBlock: false }) });
    for (const role of rolesFor(group, m)) {
      list.push({ label: $t('msg_group_member_make', { role: $t(`msg_group_role_${role}` as 'msg_group_role_admin') }), icon: 'shield',
        onselect: () => demoting(m, role, () => act({ op: 'set_role', who: m.pubkey, role })) });
    }
    if (canMute(group, m) && hasModeration(group)) {
      if (mutedNow(m)) list.push({ label: $t('msg_group_member_unmute'), icon: 'bell', onselect: () => moderate({ t: 'unmute', who: m.pubkey }) });
      else list.push({ label: $t('msg_mod_member_mute'), icon: 'bell-off', onselect: () => { dialogError = ''; mute = { open: true, m, span: 'hour' }; } });
    } else if (canMute(group, m)) {
      // A core without moderation: the mute of the log, until lifted.
      list.push({ label: m.muted ? $t('msg_group_member_unmute') : $t('msg_group_member_mute'), icon: 'bell-off', onselect: () => act({ op: 'set_muted', who: m.pubkey, muted: !m.muted }) });
    }
    if (canTrust(group, m) && checksMessages(group)) {
      list.push(m.trusted
        ? { label: $t('msg_mod_member_untrust'), icon: 'shield', onselect: () => moderate({ t: 'trust', who: m.pubkey, trusted: false }) }
        : { label: $t('msg_mod_member_trust'), icon: 'shield-check', onselect: () => moderate({ t: 'trust', who: m.pubkey, trusted: true }) });
    }
    if (isOwner(group) && !m.is_me) {
      list.push({ label: $t('msg_group_member_transfer'), icon: 'key', onselect: () => sure($t('msg_group_transfer_confirm', { name: who }), $t('msg_group_member_transfer'), { op: 'transfer_ownership', to: m.pubkey }) });
    }
    if (canRemove(group, m)) {
      list.push({ type: 'separator' });
      list.push({ label: $t('msg_group_member_remove'), icon: 'x', danger: true,
        onselect: () => demoting(m, null, () => sure($t('msg_group_remove_confirm', { name: who }), $t('msg_group_member_remove'), { op: 'remove', who: m.pubkey })) });
      if (hasModeration(group)) list.push({ label: $t('msg_mod_ban'), icon: 'user-cancel', danger: true, onselect: () => { dialogError = ''; ban = { open: true, m, hide: true }; } });
      else list.push({ label: $t('msg_group_member_ban'), icon: 'user-cancel', danger: true, onselect: () => sure($t('msg_group_ban_confirm', { name: who }), $t('msg_group_member_ban'), { op: 'ban', who: m.pubkey }) });
    }
    return list;
  }

  const items = $derived(menu.m ? entriesFor(menu.m) : []);

  const when = stamp;

  /** A key younger than this is still reaching everyone. */
  const FRESH_SECS = 24 * 3600;
  type Tone = 'ok' | 'fresh' | 'bad';
  /** Red: something is wrong with the key; yellow: it is new or still being handed out; green: settled. */
  function keyTone(k: MessengerGroupKey): Tone {
    if (!k.held || k.status === 'rotate') return 'bad';
    if (k.status === 'deliver' || Date.now() / 1000 - k.since < FRESH_SECS) return 'fresh';
    return 'ok';
  }
  function keyLabel(k: MessengerGroupKey): TranslationKey {
    if (!k.held) return 'msg_group_key_status_missing';
    if (k.status !== 'good') return `msg_group_key_status_${k.status}`;
    return keyTone(k) === 'fresh' ? 'msg_group_key_status_fresh' : 'msg_group_key_status_good';
  }
  /** Fingerprint in groups of four, the way people compare them aloud. */
  const fingerprint = (id: string) => id.match(/.{1,4}/g)?.join(' ') ?? id;

  async function leave() {
    if (await confirmStore.ask($t('msg_group_leave_confirm', { name: group.name }), $t('msg_group_leave'), true)) await act({ op: 'leave' });
  }

  async function disband() {
    if (await confirmStore.ask($t('msg_group_disband_confirm', { name: group.name }), $t('msg_group_disband'), true)) await act({ op: 'disband' });
  }

  async function forget() {
    if (!(await confirmStore.ask($t('msg_group_forget_confirm', { name: group.name }), $t('msg_group_forget'), true))) return;
    if (await run(() => groupStore.forget(group.id))) onforgotten?.();
  }

  // ── The contents and its pages ─────────────────────────────────────────
  // The panel is contents: each row opens the page of what it names, with a
  // way back in the head. What cannot be undone stays at the foot of it.

  type Page = '' | 'key' | 'link' | 'members' | 'banned';
  let opened = $state<Page>('');
  const pages = $derived<Record<Exclude<Page, ''>, boolean>>({
    key: joined && !!group.key,
    link: joined && !!group.link,
    members: true,
    banned: manager && group.banned.length > 0,
  });
  /** The page shown: none when what it showed is gone (the last ban lifted, a role taken). */
  const shown = $derived<Page>(opened && pages[opened] ? opened : '');
  const PAGE_TITLE: Record<Exclude<Page, ''>, TranslationKey> = {
    key: 'msg_group_key', link: 'msg_group_link', members: 'msg_group_members', banned: 'msg_group_banned',
  };
  // Another group starts from its contents.
  $effect(() => { void group.id; untrack(() => (opened = '')); });
  // A page whose matter is gone (the last ban lifted) is left for good: a later ban does not open it again.
  $effect(() => { if (opened && !pages[opened]) untrack(() => (opened = '')); });

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

  /** The colours of a key's state: its tile and its pill. */
  const KEY_TILE: Record<Tone, TileTone> = { ok: 'ok', fresh: 'warn', bad: 'bad' };
  const KEY_PILL: Record<Tone, PillTone> = { ok: 'ok', fresh: 'warn', bad: 'bad' };

  const chatId = $derived(`group:${group.id}`);
</script>

<!-- The key of the group: its version, cipher, state, who changed it and when, its fingerprint. -->
{#snippet keyCard(k: MessengerGroupKey)}
  {@const tone = keyTone(k)}
  <section class="box key {tone}" aria-label={$t('msg_group_key')}
    title={k.source === 'link' ? $t('msg_group_key_source_link', { n: String(k.link_epoch + 1) }) : $t('msg_group_key_source_random')}>
    <div class="key-head">
      <Tile icon="key" tone={KEY_TILE[tone]} />
      <span class="key-name">{$t('msg_group_key')} <b>v{k.version}</b></span>
      <StatePill text={$t(keyLabel(k))} tone={KEY_PILL[tone]} />
    </div>
    <p class="key-cipher">{k.cipher}</p>
    <p class="key-meta">
      {$t('msg_group_key_since')} {when(k.since)} · {$t(`msg_group_key_reason_${k.reason}` as 'msg_group_key_reason_other')} · {nameStore.label(k.by)}
    </p>
    <code class="key-fp">{fingerprint(k.id)}</code>
    {#if !k.held}
      <p class="key-note">{$t('msg_group_key_missing')}</p>
    {:else if k.status !== 'good'}
      <p class="key-note">{$t(`msg_group_key_status_${k.status}_hint` as 'msg_group_key_status_rotate_hint')}</p>
    {/if}
  </section>
{/snippet}

<!-- Who is in, in which role: a row opens what may be done about them. -->
{#snippet membersBody()}
  <section class="block" aria-label={$t('msg_group_members')}>
    {#if manager}<button class="btn btn-ghost wide" onclick={() => (inviting = true)}><Icon name="user-plus" size={18} />{$t('msg_group_invite')}</button>{/if}
    <ul class="box rows">
      {#each group.members as m (m.pubkey)}
        <li>
          <button class="person as-button" class:still={!entriesFor(m).length} oncontextmenu={(e) => openMenu(e, m)} onclick={(e) => openMenu(e, m)}
            use:longpress={{ onpress: (p) => openMenu(p, m) }}>
            <Avatar url={nameStore.picture(m.pubkey)} label={nameStore.label(m.pubkey)} seed={m.pubkey} size={36} />
            <span class="who">
              <span class="who-name">{nameStore.label(m.pubkey)}{#if m.is_me} <span class="me">({$t('msg_you')})</span>{/if}</span>
              <code>{shortKey(m.pubkey)}</code>
            </span>
            {#if m.trusted && checksMessages(group)}<span class="dim" title={$t('msg_mod_member_trusted')}><Icon name="shield-check" size={16} /></span>{/if}
            {#if mutedNow(m)}
              <span class="dim" title={m.muted_until ? $t('msg_mod_member_muted_until', { time: untilText(m.muted_until) }) : $t('msg_group_member_muted')}><Icon name="bell-off" size={16} /></span>
            {/if}
            {#if m.role !== 'member'}<span class="role {m.role}">{$t(`msg_group_role_${m.role}` as 'msg_group_role_admin')}</span>{/if}
          </button>
        </li>
      {/each}
    </ul>
  </section>
{/snippet}

{#snippet bannedBody()}
  <section class="block" aria-label={$t('msg_group_banned')}>
    <ul class="box rows">
      {#each group.banned as b (b)}
        <li class="person">
          <Avatar url={nameStore.picture(b)} label={nameStore.label(b)} seed={b} size={36} />
          <span class="who"><span class="who-name">{nameStore.label(b)}</span><code>{shortKey(b)}</code></span>
          <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => unban(b)}>{$t('msg_group_unban')}</button>
        </li>
      {/each}
    </ul>
  </section>
{/snippet}


<div class="panel" bind:this={panelEl}>
  {#if section}
  <SharedList chatId={chatId} {section} onback={() => (section = null)} />
  {:else if moderation && seesModeration(group)}
  <ModerationPanel {group} bind:tab={modTab} onback={() => (moderation = false)} />
  {:else if shown && !editing}
  <PanelHead title={$t(PAGE_TITLE[shown])} onback={closePage} onclose={close} />
  <div class="scroll">
    {#if error}<div class="error-msg">{error}</div>{/if}
    {#if shown === 'key' && group.key}{@render keyCard(group.key)}
    {:else if shown === 'link'}<GroupLink {group} heading={false} />
    {:else if shown === 'members'}{@render membersBody()}
    {:else if shown === 'banned'}{@render bannedBody()}
    {/if}
  </div>
  {:else}
  <PanelHead title={$t('msg_group_info')} onback={editing ? stopEdit : undefined} onclose={close} />

  {#if editing}
    <div class="scroll">
      <section class="edit">
        <!-- The picture takes effect at once, apart from Save: it has commands of its own. -->
        <AvatarPicker {group} disabled={busy} />
        <input class="field" type="text" bind:value={name} maxlength="64" placeholder={$t('msg_group_name')} aria-label={$t('msg_group_name')} disabled={busy} />
        <textarea class="field" rows="3" bind:value={about} maxlength="500" placeholder={$t('msg_group_about')} aria-label={$t('msg_group_about')} disabled={busy}></textarea>
        {#if group.kind === 'private'}
          <div class="option">
            <span class="option-text">{$t('msg_group_history_for_new')}</span>
            <button class="toggle" class:on={history} role="switch" aria-checked={history} aria-label={$t('msg_group_history_for_new')}
              disabled={busy} onclick={() => (history = !history)}></button>
          </div>
        {/if}
      </section>
      {#if error}<div class="error-msg">{error}</div>{/if}
    </div>
    <!-- The form's buttons stay at the foot of the panel; the form scrolls above them. -->
    <footer class="foot">
      <button class="btn btn-ghost" disabled={busy} onclick={stopEdit}>{$t('msg_group_cancel')}</button>
      <button class="btn btn-primary" disabled={busy || !name.trim()} onclick={save}>{$t('msg_group_save')}</button>
    </footer>
  {:else}
  <div class="scroll">
    <section class="hero">
      <div class="hero-row">
        {#if manager}
          <!-- The picture is changed in the editing, where its picker is. -->
          <button type="button" class="cover-btn" onclick={startEdit} aria-label={$t('msg_profile_avatar_change')} title={$t('msg_profile_avatar_change')}>
            <Avatar url={group.picture || null} label={group.name} seed={group.id} size={60} />
            <span class="cam"><Icon name="camera" size={14} /></span>
          </button>
        {:else}
          <Avatar url={group.picture || null} label={group.name} seed={group.id} size={60} />
        {/if}
        <div class="hero-text">
          <div class="name">{group.name}</div>
          <div class="kind-line">
            <Icon name={group.kind === 'public' ? 'world' : 'lock-square-rounded'} size={14} />
            <span>{$t(group.kind === 'public' ? 'msg_group_kind_public' : 'msg_group_kind_private')} ·
            {$t(membersKey(group.members.length), { n: String(group.members.length) })}</span>
          </div>
        </div>
        {#if manager}
          <button type="button" class="edit-btn" onclick={startEdit} aria-label={$t('msg_group_edit')} title={$t('msg_group_edit')}><Icon name="pencil" size={18} /></button>
        {/if}
      </div>
      {#if group.about}<div class="about"><MessageContent text={group.about} cards={false} /></div>{/if}
      <p class="fact">
        <Icon name="clock" size={14} />
        <span>
          {#if group.kind === 'public'}{$t('msg_group_note_public')}
          {:else if group.history_for_new}{$t('msg_group_note_history_on')}
          {:else}{$t('msg_group_note_history_off')}{/if}
        </span>
      </p>
    </section>

    {#if error}<div class="error-msg">{error}</div>{/if}

    {#if !joined}
      <div class="state"><Icon name="clock" size={16} />{$t(`msg_group_state_${group.membership}` as 'msg_group_state_left')}</div>
    {/if}

    {#if seesModeration(group)}
      <button class="mod-row" class:waits={!!waiting} onclick={openModeration}>
        <Tile icon="shield" tone={waiting ? 'accent' : 'plain'} />
        <span class="mod-text">
          <span class="mod-name">{$t('msg_mod_title')}</span>
          {#if waiting}<span class="mod-waiting">{waiting}</span>{:else}<span class="mod-hint">{$t('msg_mod_row_hint')}</span>{/if}
        </span>
        <Chev />
      </button>
    {/if}

    <!-- The contents: each row opens the page of what it names. -->
    {#if pages.key || pages.link}
      <div class="nav-group">
        {#if pages.key && group.key}
          {@const k = group.key}
          {@const tone = keyTone(k)}
          <NavRow icon="key" title={`${$t('msg_group_key')} v${k.version}`} page="key" nowrap tone={tone === 'ok' ? 'plain' : KEY_TILE[tone]} onclick={() => openPage('key')}>
            {#snippet sub()}{k.cipher} · {$t('msg_group_key_since')} {when(k.since)}{/snippet}
            {#snippet end()}<StatePill text={$t(keyLabel(k))} tone={KEY_PILL[tone]} />{/snippet}
          </NavRow>
        {/if}
        {#if pages.link}
          <NavRow icon="link" title={$t('msg_group_link')} page="link" onclick={() => openPage('link')}>
            {#snippet sub()}<code class="link-line">{group.link}</code>{/snippet}
          </NavRow>
        {/if}
      </div>
    {/if}

    <!-- What the group shared: the block of the panel about a person too. -->
    <SharedMedia {chatId} onopen={(s) => (section = s)} />

    <div class="nav-group">
      <NavRow icon="users" title={$t('msg_group_members')} page="members" onclick={() => openPage('members')} count={group.members.length}>
        {#snippet sub()}<span class="stack">{#each group.members.slice(0, 7) as m (m.pubkey)}<span class="stack-av"><Avatar url={nameStore.picture(m.pubkey)} label={nameStore.label(m.pubkey)} seed={m.pubkey} size={22} /></span>{/each}</span>{/snippet}
      </NavRow>
      {#if pages.banned}
        <NavRow icon="ban" title={$t('msg_group_banned')} page="banned" onclick={() => openPage('banned')} count={group.banned.length}>
          {#snippet sub()}{group.banned.map((b) => nameStore.label(b)).join(', ')}{/snippet}
        </NavRow>
      {/if}
    </div>

    <section class="zone">
      {#if isOwner(group)}<p class="hint">{$t('msg_group_owner_hint')}</p>{/if}
      <div class="zone-btns">
        {#if joined && !isOwner(group)}
          <button class="btn btn-ghost danger" disabled={busy} onclick={leave}><Icon name="x" size={18} />{$t('msg_group_leave')}</button>
        {/if}
        {#if isOwner(group)}
          <button class="btn btn-ghost danger" disabled={busy} onclick={disband}><Icon name="trash-2" size={18} />{$t('msg_group_disband')}</button>
        {/if}
        {#if !group.banned}
          <button class="btn btn-ghost danger" disabled={busy}
            onclick={() => reportStore.open({ target: 'group', accused: '', name: group.name, context: { t: 'group', group_id: group.id }, canBlock: false })}><Icon name="alert-triangle" size={18} />{$t('msg_report_group')}</button>
        {/if}
        {#if !joined && group.membership !== 'joining'}
          <button class="btn btn-ghost danger" disabled={busy} onclick={forget}><Icon name="trash-2" size={18} />{$t('msg_group_forget')}</button>
        {/if}
      </div>
    </section>
  </div>
  {/if}
  {/if}
</div>

<InvitePicker bind:open={inviting} {group} />
<ContextMenu bind:open={menu.open} x={menu.x} y={menu.y} {items} onclose={() => (menu.open = false)} />
<ChoiceDialog bind:open={mute.open} bind:value={mute.span} title={$t('msg_mod_mute_title', { name: mute.m ? nameStore.label(mute.m.pubkey) : '' })}
  options={muteOptions} cta={$t('msg_mod_mute_cta')} danger busy={dialogBusy} error={dialogError} onconfirm={confirmMute} />
<ChoiceDialog bind:open={ban.open} bind:checked={ban.hide} title={$t('msg_mod_ban_title', { name: ban.m ? nameStore.label(ban.m.pubkey) : '' })}
  text={$t('msg_mod_ban_text')} check={$t('msg_mod_ban_hide')} cta={$t('msg_mod_ban')} danger busy={dialogBusy} error={dialogError} onconfirm={confirmBan} />

<style>
  .panel { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--surface); }
  .scroll { flex: 1; min-height: 0; overflow-y: auto; padding: var(--sp-4) var(--sp-4) var(--sp-6); display: flex; flex-direction: column; gap: var(--sp-4); }
  /* A tablet or a wide window: the column stays readable in the middle. */
  .scroll > :global(*) { width: 100%; max-width: 560px; align-self: center; flex-shrink: 0; }
  .box { border-radius: var(--radius-lg); border: 1px solid var(--border); background: var(--surface-2); }
  /* A block's total is the .count-pill of base.css, .quiet. */
  .hint { margin: 0; font-size: var(--fs-sm); color: var(--text-3); line-height: 1.45; }
  .danger { color: var(--danger-text); }

  /* Who: the cover beside the name, the description, the rule of history a fact under a line. */
  .hero { display: flex; flex-direction: column; gap: var(--sp-3); }
  .hero-row { display: flex; align-items: center; gap: var(--sp-3); }
  .hero-row > :global(.avatar) { flex: none; }
  .cover-btn { position: relative; flex: none; display: inline-flex; padding: 0; border: none; background: none; border-radius: 50%; cursor: pointer; }
  .cover-btn:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  .cam {
    position: absolute; right: -2px; bottom: -2px; display: grid; place-items: center; width: 26px; height: 26px; border-radius: 50%;
    background: var(--glass-bg-raised); border: 1px solid var(--border-2); color: var(--text);
  }
  .cover-btn:hover .cam { border-color: var(--accent-border); }
  .hero-text { flex: 1; min-width: 0; display: flex; flex-direction: column; align-items: flex-start; gap: var(--sp-1); }
  .name { font-size: var(--fs-md); font-weight: var(--fw-bold); line-height: 1.25; overflow-wrap: anywhere; word-break: break-word; }
  .kind-line :global(svg) { flex: none; margin-top: 2px; }
  .kind-line { display: flex; align-items: flex-start; gap: var(--sp-1); line-height: 1.4; font-size: var(--fs-sm); color: var(--text-3); }
  .edit-btn {
    align-self: flex-start; flex: none; display: grid; place-items: center; width: 36px; height: 36px; padding: 0; border: none; border-radius: var(--radius);
    background: var(--surface-2); color: var(--text); cursor: pointer; transition: background var(--dur-fast) var(--ease);
  }
  .edit-btn:hover { background: var(--surface-hover); }
  .edit-btn:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  .about { font-size: var(--fs-base); line-height: 1.5; color: var(--text); }
  .fact {
    display: flex; align-items: flex-start; gap: var(--sp-2); margin: 0; padding-top: var(--sp-3); border-top: 1px solid var(--border-2);
    font-size: var(--fs-xs); line-height: 1.4; color: var(--text-2);
  }
  .fact :global(svg) { flex: none; margin-top: 1px; color: var(--text-3); }

  /* Editing: the form scrolls, its buttons stay at the foot. */
  .edit { display: flex; flex-direction: column; gap: var(--sp-3); }
  .field {
    width: 100%; font: inherit; font-size: var(--fs-base); color: var(--text); background: var(--surface-3);
    border: 1px solid var(--border); border-radius: var(--radius); padding: 10px var(--sp-3); resize: vertical;
  }
  input.field { min-height: var(--control-h-lg); }
  .field:focus { outline: none; border-color: var(--accent-border); }
  .option {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; padding: var(--sp-3); text-align: left;
    border-radius: var(--radius-lg); border: 1px solid var(--border); background: var(--surface-2);
  }
  .option-text { flex: 1; min-width: 0; font-size: var(--fs-base); color: var(--text); line-height: 1.4; }
  /* Its buttons line up with the form above: no wider than 560, in the middle on a tablet. */
  .foot {
    flex: none; display: grid; grid-template-columns: 1fr 1fr; gap: var(--sp-2); padding: var(--sp-3) max(var(--sp-4), calc((100% - 560px) / 2));
    padding-bottom: calc(var(--sp-3) + var(--sab, 0px));
    border-top: 1px solid var(--border); background: var(--glass-bar);
  }
  .foot .btn { min-height: var(--control-h); min-width: 0; }

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
  .mod-row:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  .mod-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .mod-name { font-size: var(--fs-body); font-weight: var(--fw-bold); }
  .mod-hint { font-size: var(--fs-xs); color: var(--text-3); line-height: 1.4; }
  .mod-row.waits { border-color: var(--accent-border); background: var(--accent-bg); }
  .mod-waiting { font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--accent-text); line-height: 1.4; }

  /* The contents: groups of rows. */
  .nav-group > :global(.nav + .nav)::before { content: ''; position: absolute; top: 0; left: 60px; right: var(--sp-3); height: 1px; background: var(--border-2); }
  .nav-group { display: flex; flex-direction: column; padding: var(--sp-1); border-radius: var(--radius-lg); border: 1px solid var(--border); background: var(--surface-2); }
  .link-line { font-size: var(--fs-2xs); color: var(--text-3); }
  .stack { display: inline-flex; padding-left: var(--sp-2); margin-top: 2px; }
  .stack-av { display: inline-flex; margin-left: calc(-1 * var(--sp-2)); border-radius: 50%; box-shadow: 0 0 0 2px var(--surface-2); }

  /* The key: green when settled, yellow while it is new or handed out, red when wrong. */
  .key {
    display: flex; flex-direction: column; gap: var(--sp-2); padding: var(--sp-4); border-color: var(--success-border);
    --tone: var(--success-text, var(--success));
  }
  .key.fresh { border-color: var(--warn-border); --tone: var(--warn-text); }
  .key.bad { border-color: var(--danger-border); --tone: var(--danger-text); }
  .key-head { display: flex; align-items: center; gap: var(--sp-3); }
  .key-name { flex: 1; min-width: 0; font-size: var(--fs-body); font-weight: var(--fw-semibold); color: var(--text); }
  .key-cipher { margin: 0; font-size: var(--fs-xs); color: var(--text-3); }
  .key-meta { margin: 0; font-size: var(--fs-sm); line-height: 1.5; color: var(--text-2); overflow-wrap: anywhere; }
  .key-fp {
    display: block; padding: var(--sp-2) var(--sp-3); border-radius: var(--radius); background: var(--surface-3);
    font-size: var(--fs-xs); color: var(--text-2); letter-spacing: 0.2px;
  }
  .key-note { margin: 0; font-size: var(--fs-sm); line-height: 1.45; color: var(--tone); }

  /* The pages of people: a card of rows. */
  .block { display: flex; flex-direction: column; gap: var(--sp-3); }
  .wide { min-height: var(--control-h); gap: var(--sp-2); justify-content: center; }
  .rows { list-style: none; margin: 0; padding: var(--sp-1); display: flex; flex-direction: column; }
  .rows > li { position: relative; }
  .rows > li + li::before { content: ''; position: absolute; top: 0; left: var(--sp-3); right: var(--sp-3); height: 1px; background: var(--border-2); }
  .person { display: flex; align-items: center; gap: var(--sp-3); min-height: 52px; padding: var(--sp-2) var(--sp-2) var(--sp-2) var(--sp-3); border-radius: var(--radius-md); min-width: 0; }
  .as-button { width: 100%; border: none; background: none; color: inherit; font: inherit; text-align: left; cursor: pointer; transition: background var(--dur-fast) var(--ease); }
  .as-button:hover { background: var(--surface-row-hover); }
  .as-button:focus-visible { outline: 2px solid var(--accent-border); outline-offset: -2px; }
  .as-button.still { cursor: default; }
  .as-button.still:hover { background: none; }
  .who { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
  .who-name { font-size: var(--fs-base); font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .who code { font-size: var(--fs-2xs); color: var(--text-3); }
  .me { color: var(--text-3); font-weight: var(--fw-medium); }
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

  @media (pointer: coarse) {
    .person { min-height: 56px; }
    .field { font-size: var(--fs-md); }
    .zone-btns .btn, .foot .btn, .wide { min-height: var(--control-h-lg); }
  }
</style>
