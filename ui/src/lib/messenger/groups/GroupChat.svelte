<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A group as a conversation: the chat window with the things only a chat
  of many has (who wrote, what happened to the group, moderation), and
  the panel with its members beside it.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { t } from '$lib/core/i18n';
  import { membersKey } from '../i18n';
  import Icon from '$lib/core/Icon.svelte';
  import type { MenuEntry } from '$lib/core/ui/ContextMenu.svelte';
  import ChatWindow from '../dm/ChatWindow.svelte';
  import GroupInfo from './GroupInfo.svelte';
  import WithInfo from '../shared/WithInfo.svelte';
  import { groupStore } from './groupStore.svelte';
  import { nameStore } from './names.svelte';
  import { groupLine } from './lines';
  import { canModerate } from './permissions';
  import { chatStore } from '../chats/chatStore.svelte';
  import { confirmStore } from '../shared/confirm.svelte';
  import { messengerError, type MessengerChat, type MessengerMessage } from '../api';
  import { groupRefusal } from './errors';

  interface Props {
    chat: MessengerChat;
    onback?: () => void;
  }
  let { chat, onback }: Props = $props();

  let info = $state(false);
  let error = $state('');
  let busy = $state(false);

  const group = $derived(groupStore.byChat(chat.id));
  const groupId = $derived(chat.id.slice(6));

  // The panel belongs to the group it was opened for.
  let last = '';
  $effect(() => {
    if (chat.id !== last) { last = chat.id; info = false; error = ''; }
    if (!groupStore.byChat(chat.id)) groupStore.refresh(chat.id.slice(6)).catch(() => {});
  });

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'msg_you', params);
  const author = (pubkey: string) => nameStore.label(pubkey);
  const systemText = (m: MessengerMessage) => { void $t; return groupLine(m, tr, author); };
  const moderate = (m: MessengerMessage) => canModerate(group, m.sender_pubkey);
  const explainError = (e: unknown) => {
    return groupRefusal(e, tr);
  };

  // What the chat itself may do is decided by the group, not by the row
  // of the chat list (which knows only whether I am in).
  const view = $derived<MessengerChat>({ ...chat, title: group?.name || chat.title, can_send: !!group?.can_post });

  async function run(fn: () => Promise<unknown>) {
    error = ''; busy = true;
    try { await fn(); }
    catch (e) { error = explainError(e) ?? messengerError(e); }
    finally { busy = false; }
  }

  async function leave() {
    if (!(await confirmStore.ask($t('msg_group_leave_confirm', { name: view.title }), $t('msg_group_leave'), true))) return;
    await run(() => groupStore.act(groupId, { op: 'leave' }));
  }

  async function forget() {
    if (!(await confirmStore.ask($t('msg_group_forget_confirm', { name: view.title }), $t('msg_group_forget'), true))) return;
    await run(async () => {
      await groupStore.forget(groupId);
      chatStore.close();
      await chatStore.loadChats();
      onback?.();
    });
  }

  const entries = $derived.by((): MenuEntry[] => {
    const list: MenuEntry[] = [{ label: $t('msg_group_info'), icon: 'users', onselect: () => (info = true) }];
    if (group?.membership === 'joined' && group.my_role !== 'owner') list.push({ label: $t('msg_group_leave'), icon: 'x', danger: true, onselect: leave });
    if (group && group.membership !== 'joined') list.push({ label: $t('msg_group_forget'), icon: 'trash-2', danger: true, onselect: forget });
    return list;
  });

  const membership = $derived(group?.membership ?? "joined");
</script>

{#snippet subtitle()}
  {#if group}
    <span class="sub-line">
      <Icon name={group.kind === 'public' ? 'globe' : 'lock'} size={11} />
      {$t(group.kind === 'public' ? 'msg_group_kind_public' : 'msg_group_kind_private')} ·
      {$t(membersKey(group.members.length), { n: String(group.members.length) })}
    </span>
  {/if}
{/snippet}

{#snippet actions()}
  {#if group && group.requests.length > 0}
    <button class="pill" onclick={() => (info = true)} title={$t('msg_group_requests')}>
      <Icon name="user-plus" size={13} />{group.requests.length}
    </button>
  {/if}
  <button class="icon" class:active={info} onclick={() => (info = !info)} title={$t('msg_group_info')}><Icon name="users" size={16} /></button>
{/snippet}

{#snippet banner()}
  {#if group && group.undecrypted > 0 && membership === 'joined'}
    <div class="strip info"><Icon name="key" size={13} />{$t('msg_group_undecrypted', { n: String(group.undecrypted) })}</div>
  {/if}
  {#if group?.muted && membership === 'joined'}
    <div class="strip warn"><Icon name="bell-off" size={13} />{$t('msg_group_you_are_muted')}</div>
  {/if}
  {#if error}<div class="strip danger">{error}</div>{/if}
{/snippet}

{#snippet footer()}
  <div class="closed">
    <span>
      <Icon name={membership === 'joining' || membership === 'requested' ? 'clock' : 'lock'} size={13} />
      {#if group?.muted && membership === 'joined'}{$t('msg_group_you_are_muted')}
      {:else}{$t(`msg_group_state_${membership}` as 'msg_group_state_left')}{/if}
    </span>
    {#if membership !== 'joined' && membership !== 'joining'}
      <button class="btn btn-ghost btn-sm" disabled={busy} onclick={forget}>{$t('msg_group_forget')}</button>
    {/if}
  </div>
{/snippet}

{#snippet window()}
  <ChatWindow chat={view} {onback} {subtitle} {actions} {banner} {footer} {author} {systemText} chatEntries={entries}
    canModerate={moderate} {explainError} ontitle={() => (info = !info)} />
{/snippet}

{#snippet panel()}
  {#if group}
    <GroupInfo {group} onclose={() => (info = false)} onforgotten={() => { info = false; chatStore.close(); chatStore.loadChats().catch(() => {}); onback?.(); }} />
  {/if}
{/snippet}

<WithInfo open={info && !!group} chat={window} info={panel} />

<style>
  .sub-line { display: inline-flex; align-items: center; gap: 4px; }
  .icon { border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; padding: 6px; border-radius: var(--radius-sm); }
  .icon:hover { color: var(--text); background: var(--surface-3); }
  .icon.active { color: var(--accent-text-2); background: var(--accent-tint); }
  .pill {
    display: inline-flex; align-items: center; gap: 4px; border: none; cursor: pointer; font: inherit; font-size: var(--fs-2xs);
    font-weight: var(--fw-bold); padding: 3px 8px; border-radius: var(--radius-pill); background: var(--accent); color: #fff;
  }
  .strip { display: flex; align-items: center; gap: 6px; padding: 6px var(--sp-4); font-size: var(--fs-xs); border-bottom: 1px solid var(--border); }
  .strip.info { color: var(--text-2); background: var(--surface-2); }
  .strip.warn { color: var(--warn-text); background: var(--warn-bg); border-color: var(--warn-border); }
  .strip.danger { color: var(--danger-text); background: var(--danger-bg); border-color: var(--danger-border); }
  .closed {
    display: flex; align-items: center; justify-content: center; gap: var(--sp-3); flex-wrap: wrap; padding: var(--sp-3);
    border-top: 1px solid var(--border); background: var(--surface); font-size: var(--fs-xs); color: var(--text-2);
  }
  .closed span { display: inline-flex; align-items: center; gap: 6px; }
  @media (pointer: coarse) { .icon { padding: 10px; } }
</style>
