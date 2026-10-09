<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Send a link, or a contact card, into a chat. A link goes out as an
  ordinary message with the link in it; the other side draws the card from
  the link. A card goes out as a card message the runtime makes: mine
  (`pubkey` null, with my phone when ticked) or another person's public
  profile, never their phone.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import MessageContent from './MessageContent.svelte';
  import { chatStore } from '../chats/chatStore.svelte';
  import { groupStore } from '../groups/groupStore.svelte';
  import { nameStore } from '../groups/names.svelte';
  import { messengerStore } from '../store.svelte';
  import { dmErrorCode, messengerError, profileErrorCode, type MessengerChat } from '../api';
  import { chatMatches, contactHaystacks } from '../shared/search';

  interface Props {
    open: boolean;
    /** The link to send, or how to get it. Not used when `card` is given. */
    link?: string | (() => Promise<string>);
    /** Send a contact card instead of a link: `pubkey` null is mine. */
    card?: { pubkey: string | null } | null;
    /** A chat the link makes no sense in (the group itself, the person whose card it is). */
    exclude?: string | null;
  }
  let { open = $bindable(), link = '', card = null, exclude = null }: Props = $props();

  let query = $state('');
  let text = $state('');
  let busy = $state(false);
  let error = $state('');
  let sent = $state<string[]>([]);
  /** My card only: my phone goes with it. */
  let includePhone = $state(false);

  const mine = $derived(!!card && card.pubkey === null);
  const myPhone = $derived(messengerStore.ownPrivate?.phone ?? null);

  let wasOpen = false;
  /** Counts the openings: a reply meant for an earlier one is dropped. */
  let session = 0;
  /** The user ticked or unticked the phone: a late load never overrides that. */
  let phoneTouched = false;
  $effect(() => {
    if (open && !wasOpen) {
      query = ''; error = ''; sent = []; includePhone = false; phoneTouched = false;
      const mySession = ++session;
      if (card) {
        text = '';
        if (card.pubkey === null) {
          const set = (v: { share_phone: boolean; phone: string | null } | null) => {
            if (mySession !== session || !open || phoneTouched) return;
            includePhone = !!v?.phone && v.share_phone;
          };
          if (messengerStore.ownPrivate) set(messengerStore.ownPrivate);
          messengerStore.loadOwnPrivate().then(set).catch(() => {});
        }
      } else {
        text = typeof link === 'string' ? link : '';
        if (typeof link !== 'string') link().then((l) => (text = l)).catch((e) => (error = messengerError(e)));
      }
      chatStore.loadChats().catch(() => {});
    }
    wasOpen = open;
  });

  const haystacks = $derived(contactHaystacks(messengerStore.contacts));
  const chats = $derived(
    chatStore.chats.filter((c) => c.can_send && !c.archived && c.id !== exclude && chatMatches(c, query, haystacks)),
  );

  /** Whose card, as the dialog shows it. */
  const who = $derived.by(() => {
    if (!card) return null;
    if (card.pubkey === null || nameStore.isMe(card.pubkey)) {
      const me = messengerStore.identity?.pubkey ?? '';
      return { pubkey: me, label: me ? nameStore.label(me) : '', picture: messengerStore.ownProfile?.picture ?? null };
    }
    return { pubkey: card.pubkey, label: nameStore.label(card.pubkey), picture: nameStore.picture(card.pubkey) };
  });

  /** My phone never goes to a public group: such a chat waits until the phone is unticked. */
  const publicGroup = (c: MessengerChat) => c.kind === 'group' && groupStore.byChat(c.id)?.kind === 'public';
  const phoneBlocked = (c: MessengerChat) => mine && includePhone && !!myPhone && publicGroup(c);

  async function send(chat: MessengerChat) {
    if (!card && !text) return;
    error = ''; busy = true;
    try {
      if (card) await chatStore.sendCard(chat, card.pubkey, mine && includePhone && !!myPhone);
      else await chatStore.sendTo(chat, text);
      sent = [...sent, chat.id];
    } catch (e) {
      const code = profileErrorCode(e);
      const dm = code ? null : dmErrorCode(e);
      error = code ? $t(`msg_err_${code}` as 'msg_err_phone_public_group')
        : dm ? $t(`msg_err_${dm}` as 'msg_err_dm_blocked', { name: chat.title })
        : messengerError(e);
    } finally { busy = false; }
  }

  const title = $derived(!card ? $t('msg_share_title') : mine ? $t('msg_profile_send_card') : $t('msg_profile_share_card'));
</script>

<Dialog bind:open {title} width="min(440px, calc(100vw - 24px))">
  <div class="body">
    {#if card && who}
      <div class="whose">
        <Avatar url={who.picture} label={who.label} seed={who.pubkey} size={40} />
        <div class="whose-text">
          <span class="whose-name">{who.label}</span>
          <span class="whose-hint">{mine ? $t('msg_profile_send_card_hint') : $t('msg_profile_share_card_hint')}</span>
        </div>
      </div>
      {#if mine}
        {#if myPhone}
          <label class="phone">
            <input type="checkbox" bind:checked={includePhone} disabled={busy} onchange={() => (phoneTouched = true)} />
            <span class="phone-text">
              <span>{$t('msg_profile_send_card_include_phone')} <span class="num">{myPhone}</span></span>
              <span class="phone-hint">{$t('msg_profile_send_card_phone_hint')}</span>
            </span>
          </label>
        {:else if messengerStore.ownPrivate}
          <p class="hint">{$t('msg_profile_send_card_no_phone')}</p>
        {/if}
      {/if}
    {:else if text}<div class="what"><MessageContent {text} /></div>{/if}
    <input type="text" class="search" bind:value={query} placeholder={$t('msg_share_search')} spellcheck="false" />
    {#if error}<div class="error-msg">{error}</div>{/if}
    {#if chats.length}
      <ul class="list">
        {#each chats as c (c.id)}
          {@const done = sent.includes(c.id)}
          {@const blocked = phoneBlocked(c)}
          <li>
            <button class="row" disabled={busy || done || blocked || (!card && !text)} onclick={() => send(c)}
              title={blocked ? $t('msg_profile_send_card_public_group') : undefined}>
              <Avatar url={c.picture} label={c.title} seed={c.peer_pubkey ?? c.id} size={34} />
              <span class="name-col">
                <span class="name">{c.title}</span>
                {#if blocked}<span class="why">{$t('msg_profile_send_card_public_group')}</span>{/if}
              </span>
              {#if c.kind === 'group'}<span class="dim"><Icon name="users" size={12} /></span>{/if}
              <span class="state" class:done><Icon name={done ? 'check' : blocked ? 'lock' : 'send'} size={13} />{done ? $t('msg_share_sent') : ''}</span>
            </button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="hint">{$t('msg_share_empty')}</p>
    {/if}
  </div>
</Dialog>

<style>
  .body { display: flex; flex-direction: column; gap: var(--sp-3); }
  .what { display: flex; flex-direction: column; gap: 4px; pointer-events: none; }
  .search {
    font: inherit; font-size: var(--fs-sm); color: var(--text); width: 100%;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-field); padding: 9px 12px;
  }
  .search:focus { outline: none; border-color: var(--accent-border); }
  .whose {
    display: flex; align-items: center; gap: var(--sp-3); min-width: 0;
    padding: var(--sp-3); border-radius: var(--radius-md); border: 1px solid var(--border); background: var(--surface-2);
  }
  .whose-text { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .whose-name { font-size: var(--fs-sm); font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .whose-hint { font-size: var(--fs-2xs); color: var(--text-3); line-height: 1.4; }
  .phone { display: flex; align-items: flex-start; gap: var(--sp-2); cursor: pointer; font-size: var(--fs-sm); }
  .phone input { margin-top: 3px; accent-color: var(--accent); flex-shrink: 0; }
  .phone-text { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .num { font-variant-numeric: tabular-nums; color: var(--text-2); white-space: nowrap; }
  .phone-hint { font-size: var(--fs-2xs); color: var(--text-3); line-height: 1.4; }
  .list { list-style: none; margin: 0; padding: 0; max-height: 300px; overflow-y: auto; display: flex; flex-direction: column; gap: 2px; }
  .row {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; padding: 6px 8px; text-align: left;
    border: none; border-radius: var(--radius-sm); background: none; color: inherit; font: inherit; cursor: pointer;
  }
  .row:hover:not(:disabled) { background: var(--surface-row-hover); }
  .row:disabled { cursor: default; }
  .name-col { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-semibold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .why { font-size: var(--fs-2xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dim { color: var(--text-3); display: inline-flex; }
  .state { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-2xs); color: var(--accent-text-2); flex-shrink: 0; }
  .state.done { color: var(--success-text, var(--success)); }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.5; }
  @media (pointer: coarse) { .search { font-size: 16px; } .row { padding: 9px 8px; } .phone input { width: 18px; height: 18px; } }
</style>
