<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Everything about a group except its conversation: who is in, in which
  role, who asks to be let in, the link, the settings, and the way out.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { t, type TranslationKey } from '$lib/core/i18n';
  import { membersKey } from '../i18n';
  import { groupError } from './errors';
  import Icon from '$lib/core/Icon.svelte';
  import PanelHead from '../shared/PanelHead.svelte';
  import ContextMenu, { type MenuEntry } from '$lib/core/ui/ContextMenu.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import GroupLink from './GroupLink.svelte';
  import MessageContent from '../content/MessageContent.svelte';
  import InvitePicker from './InvitePicker.svelte';
  import SharedMedia from '../content/shared/SharedMedia.svelte';
  import SharedList from '../content/shared/SharedList.svelte';
  import { groupStore } from './groupStore.svelte';
  import { nameStore, shortKey } from './names.svelte';
  import { canMute, canRemove, isManager, isOwner, rolesFor } from './permissions';
  import { chatStore } from '../chats/chatStore.svelte';
  import { confirmStore } from '../shared/confirm.svelte';
  import { longpress } from '../shared/longpress';
  import { stamp } from '../shared/time';
  import { type GroupAction, type MessengerGroup, type MessengerGroupKey, type MessengerGroupMember, type SharedSection } from '../api';

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as "msg_you", params);

  interface Props {
    group: MessengerGroup;
    onclose: () => void;
    onforgotten?: () => void;
  }
  let { group, onclose, onforgotten }: Props = $props();

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

  async function sure(text: string, cta: string, a: GroupAction) {
    if (await confirmStore.ask(text, cta, true)) await act(a);
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
  }

  async function save() {
    const a: GroupAction = { op: 'edit_settings' };
    if (name.trim() && name.trim() !== group.name) a.name = name.trim();
    if (about !== group.about) a.about = about;
    if (group.kind === 'private' && history !== group.history_for_new) a.history_for_new = history;
    if (Object.keys(a).length === 1) { editing = false; return; }
    if (await act(a)) { editing = false; chatStore.scheduleChatsRefresh(); }
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
    for (const role of rolesFor(group, m)) {
      list.push({ label: $t('msg_group_member_make', { role: $t(`msg_group_role_${role}` as 'msg_group_role_admin') }), icon: 'shield', onselect: () => act({ op: 'set_role', who: m.pubkey, role }) });
    }
    if (canMute(group, m)) {
      list.push({ label: m.muted ? $t('msg_group_member_unmute') : $t('msg_group_member_mute'), icon: 'bell-off', onselect: () => act({ op: 'set_muted', who: m.pubkey, muted: !m.muted }) });
    }
    if (isOwner(group) && !m.is_me) {
      list.push({ label: $t('msg_group_member_transfer'), icon: 'key', onselect: () => sure($t('msg_group_transfer_confirm', { name: who }), $t('msg_group_member_transfer'), { op: 'transfer_ownership', to: m.pubkey }) });
    }
    if (canRemove(group, m)) {
      list.push({ type: 'separator' });
      list.push({ label: $t('msg_group_member_remove'), icon: 'x', danger: true, onselect: () => sure($t('msg_group_remove_confirm', { name: who }), $t('msg_group_member_remove'), { op: 'remove', who: m.pubkey }) });
      list.push({ label: $t('msg_group_member_ban'), icon: 'ban', danger: true, onselect: () => sure($t('msg_group_ban_confirm', { name: who }), $t('msg_group_member_ban'), { op: 'ban', who: m.pubkey }) });
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
</script>

<div class="panel">
  {#if section}
  <SharedList chatId={`group:${group.id}`} {section} onback={() => (section = null)} />
  {:else}
  <PanelHead title={$t('msg_group_info')} {onclose} />

  <div class="scroll">
    <section class="card-top">
      <Avatar url={group.picture || null} label={group.name} seed={group.id} size={64} />
      {#if editing}
        <input class="field" type="text" bind:value={name} maxlength="64" placeholder={$t('msg_group_name')} disabled={busy} />
        <textarea class="field" rows="3" bind:value={about} maxlength="500" placeholder={$t('msg_group_about')} disabled={busy}></textarea>
        {#if group.kind === 'private'}
          <div class="option">
            <span class="option-text">{$t('msg_group_history_for_new')}</span>
            <button class="toggle" class:on={history} role="switch" aria-checked={history} aria-label={$t('msg_group_history_for_new')}
              disabled={busy} onclick={() => (history = !history)}></button>
          </div>
        {/if}
        <div class="row">
          <button class="btn btn-primary btn-sm" disabled={busy || !name.trim()} onclick={save}>{$t('msg_group_save')}</button>
          <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => (editing = false)}>{$t('msg_group_cancel')}</button>
        </div>
      {:else}
        <div class="name">{group.name}</div>
        <div class="kind">
          <Icon name={group.kind === 'public' ? 'globe' : 'lock'} size={12} />
          {$t(group.kind === 'public' ? 'msg_group_kind_public' : 'msg_group_kind_private')} ·
          {$t(membersKey(group.members.length), { n: String(group.members.length) })}
        </div>
        {#if group.about}<div class="about"><MessageContent text={group.about} cards={false} /></div>{/if}
        <p class="note">
          {#if group.kind === 'public'}{$t('msg_group_note_public')}
          {:else if group.history_for_new}{$t('msg_group_note_history_on')}
          {:else}{$t('msg_group_note_history_off')}{/if}
        </p>
        {#if manager}<button class="btn btn-ghost btn-sm" onclick={startEdit}><Icon name="pencil" size={13} />{$t('msg_group_edit')}</button>{/if}
      {/if}
    </section>

    {#if joined && group.key}
      {@const k = group.key}
      {@const tone = keyTone(k)}
      <section class="key {tone}" aria-label={$t('msg_group_key')}
        title={k.source === 'link' ? $t('msg_group_key_source_link', { n: String(k.link_epoch + 1) }) : $t('msg_group_key_source_random')}>
        <div class="key-head">
          <Icon name="key" size={13} />
          <span class="key-name">{$t('msg_group_key')} <b>v{k.version}</b></span>
          <span class="key-cipher">{k.cipher}</span>
          <span class="key-status">{$t(keyLabel(k))}</span>
        </div>
        <div class="key-meta">
          {$t('msg_group_key_since')} {when(k.since)} · {$t(`msg_group_key_reason_${k.reason}` as 'msg_group_key_reason_other')} · {nameStore.label(k.by)}
        </div>
        <code class="key-fp">{fingerprint(k.id)}</code>
        {#if !k.held}
          <p class="key-note">{$t('msg_group_key_missing')}</p>
        {:else if k.status !== 'good'}
          <p class="key-note">{$t(`msg_group_key_status_${k.status}_hint` as 'msg_group_key_status_rotate_hint')}</p>
        {/if}
      </section>
    {/if}

    {#if error}<div class="error-msg">{error}</div>{/if}

    {#if !joined}
      <div class="state">{$t(`msg_group_state_${group.membership}` as 'msg_group_state_left')}</div>
    {/if}

    {#if joined && group.requests.length}
      <section class="block">
        <div class="label">{$t('msg_group_requests')} · {group.requests.length}</div>
        <ul class="list">
          {#each group.requests as r (r)}
            <li class="person">
              <Avatar url={nameStore.picture(r)} label={nameStore.label(r)} seed={r} size={34} />
              <span class="who"><span class="who-name">{nameStore.label(r)}</span><code>{shortKey(r)}</code></span>
              <button class="btn btn-primary btn-sm" disabled={busy} onclick={() => run(() => groupStore.answerRequest(group.id, r, true))}>{$t('msg_group_request_approve')}</button>
              <button class="icon" disabled={busy} title={$t('msg_group_request_reject')} onclick={() => run(() => groupStore.answerRequest(group.id, r, false))}><Icon name="x" size={15} /></button>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    {#if joined}<GroupLink {group} />{/if}

    <SharedMedia chatId={`group:${group.id}`} onopen={(s) => (section = s)} />

    <section class="block">
      <div class="label-row">
        <span class="label">{$t('msg_group_members')} · {group.members.length}</span>
        {#if manager}<button class="btn btn-ghost btn-sm" onclick={() => (inviting = true)}><Icon name="user-plus" size={13} />{$t('msg_group_invite')}</button>{/if}
      </div>
      <ul class="list">
        {#each group.members as m (m.pubkey)}
          <li>
            <button class="person as-button" class:still={!entriesFor(m).length} oncontextmenu={(e) => openMenu(e, m)} onclick={(e) => openMenu(e, m)}
              use:longpress={{ onpress: (p) => openMenu(p, m) }}>
              <Avatar url={nameStore.picture(m.pubkey)} label={nameStore.label(m.pubkey)} seed={m.pubkey} size={34} />
              <span class="who">
                <span class="who-name">{nameStore.label(m.pubkey)}{#if m.is_me} <span class="me">({$t('msg_you')})</span>{/if}</span>
                <code>{shortKey(m.pubkey)}</code>
              </span>
              {#if m.muted}<span class="dim" title={$t('msg_group_member_muted')}><Icon name="bell-off" size={13} /></span>{/if}
              {#if m.role !== 'member'}<span class="role {m.role}">{$t(`msg_group_role_${m.role}` as 'msg_group_role_admin')}</span>{/if}
            </button>
          </li>
        {/each}
      </ul>
    </section>

    {#if manager && group.banned.length}
      <section class="block">
        <div class="label">{$t('msg_group_banned')} · {group.banned.length}</div>
        <ul class="list">
          {#each group.banned as b (b)}
            <li class="person">
              <Avatar url={nameStore.picture(b)} label={nameStore.label(b)} seed={b} size={34} />
              <span class="who"><span class="who-name">{nameStore.label(b)}</span><code>{shortKey(b)}</code></span>
              <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => unban(b)}>{$t('msg_group_unban')}</button>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    <section class="block out">
      {#if joined && !isOwner(group)}
        <button class="btn btn-ghost btn-sm danger" disabled={busy} onclick={leave}><Icon name="x" size={13} />{$t('msg_group_leave')}</button>
      {/if}
      {#if isOwner(group)}
        <p class="hint">{$t('msg_group_owner_hint')}</p>
        <button class="btn btn-ghost btn-sm danger" disabled={busy} onclick={disband}><Icon name="trash-2" size={13} />{$t('msg_group_disband')}</button>
      {/if}
      {#if !joined && group.membership !== 'joining'}
        <button class="btn btn-ghost btn-sm danger" disabled={busy} onclick={forget}><Icon name="trash-2" size={13} />{$t('msg_group_forget')}</button>
      {/if}
    </section>
  </div>
  {/if}
</div>

<InvitePicker bind:open={inviting} {group} />
<ContextMenu bind:open={menu.open} x={menu.x} y={menu.y} {items} onclose={() => (menu.open = false)} />

<style>
  .panel { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--surface); }
  .scroll { flex: 1; min-height: 0; overflow-y: auto; padding: var(--sp-4); display: flex; flex-direction: column; gap: var(--sp-5); }
  .card-top { display: flex; flex-direction: column; align-items: center; gap: var(--sp-2); text-align: center; }
  .name { font-size: var(--fs-md); font-weight: var(--fw-extrabold); letter-spacing: -0.2px; overflow-wrap: anywhere; word-break: break-word; }
  .kind { display: inline-flex; align-items: center; gap: 5px; font-size: var(--fs-xs); color: var(--text-3); }
  .about { color: var(--text-body); }
  .note, .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.45; }
  .field {
    width: 100%; font: inherit; font-size: var(--fs-sm); color: var(--text); background: var(--surface-2);
    border: 1px solid var(--border); border-radius: var(--radius-field); padding: 8px 10px; resize: vertical;
  }
  .field:focus { outline: none; border-color: var(--accent-border); }
  .option { display: flex; align-items: center; gap: var(--sp-3); width: 100%; text-align: left; }
  .option-text { flex: 1; min-width: 0; font-size: var(--fs-xs); color: var(--text-2); line-height: 1.4; }
  .row { display: flex; gap: 6px; flex-wrap: wrap; justify-content: center; }
  .state { padding: var(--sp-2) var(--sp-3); border-radius: var(--radius-md); background: var(--warn-bg); border: 1px solid var(--warn-border); color: var(--warn-text); font-size: var(--fs-xs); line-height: 1.45; }
  .block { display: flex; flex-direction: column; gap: var(--sp-2); }
  .block.out { align-items: flex-start; }
  .label { font-size: var(--fs-2xs); text-transform: uppercase; letter-spacing: 0.6px; color: var(--text-3); font-weight: var(--fw-bold); }
  .label-row { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .person { display: flex; align-items: center; gap: var(--sp-3); padding: 6px 8px; border-radius: var(--radius-sm); min-width: 0; }
  .as-button { width: 100%; border: none; background: none; color: inherit; font: inherit; text-align: left; cursor: pointer; }
  .as-button:hover { background: var(--surface-row-hover); }
  .as-button.still { cursor: default; }
  .as-button.still:hover { background: none; }
  .who { display: flex; flex-direction: column; gap: 1px; min-width: 0; flex: 1; }
  .who-name { font-size: var(--fs-sm); font-weight: var(--fw-semibold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .who code { font-family: var(--font-mono); font-size: var(--fs-2xs); color: var(--text-3); }
  .me { color: var(--text-3); font-weight: var(--fw-normal, 400); }
  .dim { color: var(--text-3); display: inline-flex; }
  .role { flex-shrink: 0; font-size: var(--fs-2xs); font-weight: var(--fw-bold); padding: 2px 7px; border-radius: var(--radius-pill); background: var(--surface-3); color: var(--text-2); }
  .role.owner { background: var(--accent-tint); color: var(--accent-text-2); }
  .role.admin { background: var(--success-bg); color: var(--success-text, var(--success)); }
  .key { display: flex; flex-direction: column; gap: 3px; padding: var(--sp-2) var(--sp-3); border-radius: var(--radius-md); border: 1px solid var(--success-border); background: var(--success-bg); --tone: var(--success-text, var(--success)); font-size: var(--fs-xs); line-height: 1.4; }
  .key.fresh { border-color: var(--warn-border); background: var(--warn-bg); --tone: var(--warn-text); }
  .key.bad { border-color: var(--danger-border); background: var(--danger-bg); --tone: var(--danger-text); }
  .key-head { display: flex; flex-wrap: wrap; align-items: center; gap: 2px 6px; color: var(--tone); }
  .key-name { color: var(--text); font-weight: var(--fw-semibold); }
  .key-cipher { color: var(--text-3); font-size: var(--fs-2xs); }
  .key-status { margin-left: auto; flex-shrink: 0; font-size: var(--fs-2xs); font-weight: var(--fw-bold); color: var(--tone); }
  .key-meta { color: var(--text-2); overflow-wrap: anywhere; }
  .key-fp { font-family: var(--font-mono); font-size: var(--fs-2xs); color: var(--text-3); letter-spacing: 0.2px; }
  .key-note { margin: 2px 0 0; color: var(--tone); }
  .icon { border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; padding: 6px; border-radius: var(--radius-sm); }
  .icon:hover { color: var(--text); background: var(--surface-3); }
  .danger { color: var(--danger-text); }
  @media (pointer: coarse) {
    .icon { padding: 10px; }
    .person { padding: 9px 8px; }
    .field { font-size: 16px; }
  }
</style>
