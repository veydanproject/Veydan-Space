<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A group as a conversation: the chat window with the things only a chat
  of many has (who wrote, what happened to the group, moderation), and
  the panel with its members beside it.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { locale, localeTag, t } from '$lib/core/i18n';
  import { membersKey } from '../i18n';
  import Icon from '$lib/core/Icon.svelte';
  import type { MenuEntry } from '$lib/core/ui/ContextMenu.svelte';
  import ChatWindow from '../dm/ChatWindow.svelte';
  import GroupInfo from './GroupInfo.svelte';
  import WithInfo from '../shared/WithInfo.svelte';
  import { groupStore } from './groupStore.svelte';
  import { nameStore } from './names.svelte';
  import { groupLine } from './lines';
  import { canApprove, canHide, canModerate, canSpam, hasModeration, isBannedGroup, isModerator } from './permissions';
  import ChoiceDialog from './moderation/ChoiceDialog.svelte';
  import { deviceZone, HIDE_SPANS, hideFrom, opensAt, zoneHint, type HideSpan } from './moderation/schedule';
  import { chatStore } from '../chats/chatStore.svelte';
  import { confirmStore } from '../shared/confirm.svelte';
  import { reportStore } from '../shared/report.svelte';
  import { onPhone } from '../shared/phone';
  import { messengerApi, messengerError, type MessengerChat, type MessengerMessage, type ModAction } from '../api';
  import { groupRefusal, moderationError } from './errors';
  import GroupCallButton from '../calls/GroupCallButton.svelte';
  import GroupCallBanner from '../calls/GroupCallBanner.svelte';
  import { groupCallStore } from '../calls/groupCallStore.svelte';

  interface Props {
    chat: MessengerChat;
    onback?: () => void;
  }
  let { chat, onback }: Props = $props();

  /** A phone's bar has no room for one more count: there the queue is in the chat menu. */
  const phone = onPhone();
  let info = $state(false);
  /** The panel shows the moderation of the group. */
  let moderation = $state(false);
  let error = $state('');
  let busy = $state(false);

  const group = $derived(groupStore.byChat(chat.id));
  const groupId = $derived(chat.id.slice(6));

  // The panel belongs to the group it was opened for.
  let last = '';
  $effect(() => {
    if (chat.id !== last) { last = chat.id; info = false; moderation = false; error = ''; }
    if (!groupStore.byChat(chat.id)) groupStore.refresh(chat.id.slice(6)).catch(() => {});
  });

  // The call on in the group, for its banner: the events tell of changes, the
  // runtime of what was on before the page heard any.
  let asked = '';
  $effect(() => {
    if (groupId === asked || group?.membership !== 'joined') return;
    asked = groupId;
    groupCallStore.load(groupId).catch(() => {});
  });

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'msg_you', params);
  const author = (pubkey: string) => nameStore.label(pubkey);
  const systemText = (m: MessengerMessage) => { void $t; return groupLine(m, tr, author); };
  const explainError = (e: unknown) => {
    return groupRefusal(e, tr);
  };

  // What the chat itself may do is decided by the group, not by the row
  // of the chat list (which knows only whether I am in).
  const banned = $derived(isBannedGroup(chat));
  const view = $derived<MessengerChat>({ ...chat, title: group?.name || chat.title, can_send: !!group?.can_post && !banned });

  async function run(fn: () => Promise<unknown>) {
    error = ''; busy = true;
    try { await fn(); }
    catch (e) { error = explainError(e) ?? messengerError(e); }
    finally { busy = false; }
  }

  // ── What a moderator does with a message ────────────────────────────────

  const now = () => Math.floor(Date.now() / 1000);

  async function decide(a: ModAction) {
    error = ''; busy = true;
    try {
      await messengerApi.moderation.groupMod(groupId, a);
      // The events say the same a moment later; the one who decided sees it at once.
      await Promise.all([chatStore.reloadWindow(), groupStore.refresh(groupId)]);
    } catch (e) {
      error = moderationError(e, tr);
    } finally {
      busy = false;
    }
  }

  async function spam(m: MessengerMessage) {
    if (!(await confirmStore.ask($t('msg_mod_spam_confirm', { names: author(m.sender_pubkey) }), $t('msg_mod_spam'), true))) return;
    await decide({ t: 'spam', author: m.sender_pubkey, target: m.id });
  }

  let hiding = $state<{ open: boolean; author: string; span: string }>({ open: false, author: '', span: 'hour' });
  const hideOptions = $derived(HIDE_SPANS.map((v) => ({ value: v, label: $t(`msg_mod_hide_span_${v}` as 'msg_mod_hide_span_hour') })));

  async function hideAuthor() {
    const t0 = now();
    hiding.open = false;
    await decide({ t: 'hide_author', author: hiding.author, from: hideFrom(hiding.span as HideSpan, t0), to: t0 });
  }

  /** A moderator's entries of a message: a decision on one under review, else hiding. Never on my own; none on a core that keeps no marks. */
  function modEntries(m: MessengerMessage) {
    const list: MenuEntry[] = [];
    if (!hasModeration(group) || m.direction === 'out' || m.content_type === 'system') return list;
    if (m.review === 'pending' && canApprove(group)) {
      list.push({ label: $t('msg_mod_approve'), icon: 'check', onselect: () => decide({ t: 'approve', target: m.id, version: m.version ?? m.id }) });
      list.push({ label: $t('msg_mod_reject'), icon: 'x', onselect: () => decide({ t: 'reject', target: m.id }) });
      if (canSpam(group, m.sender_pubkey)) list.push({ label: $t('msg_mod_spam'), icon: 'ban', danger: true, onselect: () => spam(m) });
    } else if (canHide(group, m.sender_pubkey)) {
      list.push({ label: $t('msg_mod_hide'), icon: 'eye-off', onselect: () => decide({ t: 'hide', targets: [m.id] }) });
      list.push({ label: $t('msg_mod_hide_author'), icon: 'eye-off', onselect: () => { hiding = { open: true, author: m.sender_pubkey, span: 'hour' }; } });
    }
    return list;
  }

  /** On a core without marks a moderator removes for everyone, as before: the one operation of the log for it. */
  const moderate = (m: MessengerMessage) => !hasModeration(group) && canModerate(group, m.sender_pubkey);

  // ── Why I cannot write, and when that ends ─────────────────────────────

  const me = $derived(group?.members.find((m) => m.is_me) ?? null);

  /** `18:00`, `пт 08:00`, with the group's zone when it is not this device's. */
  function when(at: number, tz?: string): string | null {
    const o = opensAt(at, now(), localeTag($locale), tz);
    if (!o) return null;
    const zone = tz && tz !== deviceZone() ? ` (${tz})` : '';
    return `${o.day ? `${o.day} ` : ''}${o.time}${zone}`;
  }

  const mutedText = $derived.by(() => {
    if (!group?.muted) return null;
    const until = me?.muted_until ? when(me.muted_until) : null;
    return until ? $t('msg_mod_you_muted_until', { time: until }) : $t('msg_group_you_are_muted');
  });

  const closedText = $derived.by(() => {
    if (!group) return null;
    if (group.closed_until !== null && group.closed_until !== undefined) {
      const until = when(group.closed_until, group.schedule?.tz);
      return until ? $t('msg_mod_closed_until', { time: until }) : $t('msg_mod_closed');
    }
    // «Only admins» has no hour it opens at, so the core sends no time
    // for it: the mode itself is the word, for a member who may not post.
    if (group.membership === 'joined' && !group.can_post && !group.muted && group.post_mode === 'managers' && !isModerator(group)) return $t('msg_mod_closed');
    return null;
  });

  /** The zone of the hours is one this app does not know: the hours are read in UTC meanwhile. */
  const zoneText = $derived(group?.membership === 'joined' ? zoneHint(group) : null);

  // The group is read again when a timed mute or the closed hours end: no event tells of a clock.
  $effect(() => {
    const t0 = now();
    const ends = [group?.closed_until, group?.muted ? me?.muted_until : null].filter((x): x is number => !!x && x > t0 && x - t0 < 7 * 86400);
    if (!ends.length) return;
    const id = groupId;
    const timer = setTimeout(() => groupStore.refresh(id).catch(() => {}), (Math.min(...ends) - t0 + 1) * 1000);
    return () => clearTimeout(timer);
  });

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

  /** On a phone the header keeps its plate free: the calls are entries of its menu. */
  let callButton = $state<ReturnType<typeof GroupCallButton>>();

  const entries = $derived.by((): MenuEntry[] => {
    const list: MenuEntry[] = [];
    if (phone && callButton?.canStart()) {
      const blocked = !!callButton.startBlocked();
      list.push(
        { label: $t('msg_gcall_video_start'), icon: 'video', disabled: blocked, onselect: () => callButton?.start('video') },
        { label: $t('msg_gcall_start'), icon: 'phone', disabled: blocked, onselect: () => callButton?.start('audio') },
        { type: 'separator' },
      );
    }
    list.push({ label: $t('msg_group_info'), icon: 'users', onselect: () => { moderation = false; info = true; } });
    if (isModerator(group) && hasModeration(group)) list.push({ label: $t('msg_mod_title'), icon: 'shield', onselect: () => { moderation = true; info = true; } });
    // A closed group is not reported again: the network has already acted.
    if (group && !banned) list.push({ label: $t('msg_report_group'), icon: 'alert-triangle', danger: true,
      onselect: () => reportStore.open({ target: 'group', accused: '', name: view.title, context: { t: 'group', group_id: groupId }, canBlock: false }) });
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
  <GroupCallButton bind:this={callButton} {chat} {group} menuStarts={phone} />
  {#if !phone && isModerator(group) && group.queue}
    <button class="pill" onclick={() => { moderation = true; info = true; }} title={$t('msg_mod_title')}>
      <Icon name="shield" size={13} />{group.queue}
    </button>
  {/if}
  {#if group && group.requests.length > 0}
    <button class="pill" onclick={() => { moderation = false; info = true; }} title={$t('msg_group_requests')}>
      <Icon name="user-plus" size={13} />{group.requests.length}
    </button>
  {/if}
{/snippet}

{#snippet banner()}
  {#if banned}<div class="strip warn" role="status"><Icon name="alert-triangle" size={13} />{$t('msg_group_removed')}</div>{/if}
  {#if membership === 'joined'}<GroupCallBanner {groupId} />{/if}
  {#if group && group.undecrypted > 0 && membership === 'joined'}
    <div class="strip info"><Icon name="key" size={13} />{$t('msg_group_undecrypted', { n: String(group.undecrypted) })}</div>
  {/if}
  {#if group?.needs_update && membership === 'joined'}
    <div class="strip warn"><Icon name="download" size={13} />{$t('msg_mod_needs_update')}</div>
  {/if}
  {#if mutedText && membership === 'joined'}
    <div class="strip warn"><Icon name="bell-off" size={13} />{mutedText}</div>
  {/if}
  {#if zoneText}
    <div class="strip warn"><Icon name="clock" size={13} />{$t('msg_mod_schedule_tz_unknown', { tz: zoneText })}</div>
  {/if}
  {#if error}<div class="strip danger">{error}</div>{/if}
{/snippet}

{#snippet footer()}
  <div class="closed">
    <span>
      {#if banned}
        <Icon name="lock" size={13} />{$t('msg_group_removed_short')}
      {:else}
      <Icon name={membership === 'joining' || membership === 'requested' || (closedText && membership === 'joined') ? 'clock' : 'lock'} size={13} />
      {#if mutedText && membership === 'joined'}{mutedText}
      {:else if closedText && membership === 'joined'}{closedText}
      {:else}{$t(`msg_group_state_${membership}` as 'msg_group_state_left')}{/if}
      {/if}
    </span>
    {#if membership !== 'joined' && membership !== 'joining'}
      <button class="btn btn-ghost btn-sm" disabled={busy} onclick={forget}>{$t('msg_group_forget')}</button>
    {/if}
  </div>
{/snippet}

{#snippet window()}
  <ChatWindow chat={view} {onback} {subtitle} {actions} {banner} {footer} {author} {systemText} chatEntries={entries}
    extraEntries={modEntries} canModerate={moderate} moreHistory={membership === 'joined'} {explainError} ontitle={() => { moderation = false; info = !info; }} />
{/snippet}

{#snippet panel()}
  {#if group}
    <GroupInfo {group} bind:moderation onclose={() => (info = false)} onforgotten={() => { info = false; chatStore.close(); chatStore.loadChats().catch(() => {}); onback?.(); }} />
  {/if}
{/snippet}

<WithInfo open={info && !!group} chat={window} info={panel} />
<ChoiceDialog bind:open={hiding.open} bind:value={hiding.span} title={$t('msg_mod_hide_author_title', { name: hiding.author ? author(hiding.author) : '' })}
  options={hideOptions} cta={$t('msg_mod_hide')} danger {busy} onconfirm={hideAuthor} />

<style>
  .sub-line { display: inline-flex; align-items: center; gap: 4px; }
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
</style>
