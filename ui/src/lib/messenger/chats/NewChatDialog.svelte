<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A new chat: a contact by name, a key, or a person's link. A link with an
  invitation in it carries its secret, and the request that goes with it
  is let in by the other side at once. A code of an invitation is said
  aloud with the key: it goes together with the first message, so that
  message lands in a chat the code has opened. Without a code or a secret
  the chat opens as before, and what I write first is an ordinary request.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import { messengerStore } from '../store.svelte';
  import { contactLabel, dmErrorCode, messengerApi, messengerError, type MessengerContact } from '../api';
  import type { InternalLinkText } from '../content/types';
  import { chatStore } from './chatStore.svelte';
  import { contactHaystacks, matchesQuery } from '../shared/search';
  import { inviteStore } from '../invites/inviteStore.svelte';
  import { addressKind, invitedContact, normalizeCode, type InvitedContact } from '../invites/invites';
  import { goto } from '$app/navigation';
  import { contactsHref, qrHref } from '../mobile/routes';
  import { acceptServer } from '../servers/serverAccept.svelte';
  import ServerBox from '../servers/ServerBox.svelte';
  import { serverStore } from '../servers/serverStore.svelte';
  import { privateInvite } from '../servers/privateInvite.svelte';


  interface Props {
    open: boolean;
    onopened?: () => void;
    /** A phone: the contacts and the QR codes are reached from here (the list has no button of its own for them). */
    shortcuts?: boolean;
  }
  let { open = $bindable(), onopened, shortcuts = false }: Props = $props();

  function go(href: string) {
    open = false;
    void goto(href);
  }

  let key = $state('');
  let busy = $state(false);
  let error = $state('');
  /** The line «Есть код приглашения?» was opened. */
  let codeOpen = $state(false);
  let code = $state('');
  let text = $state('');
  /** What the runtime read in a pasted link: the person, and the secret when it carries one. */
  let link = $state<InvitedContact | null>(null);
  let linkError = $state('');

  const q = $derived(key.trim());
  const kind = $derived(addressKind(q));
  const looksLikeKey = $derived(kind === 'key');
  const haystacks = $derived(contactHaystacks(messengerStore.contacts));
  const contacts = $derived(kind ? [] : messengerStore.contacts.filter((c) => matchesQuery(haystacks.get(c.pubkey) ?? '', q)));
  /** Whom the chat is with: a typed key, or the person of a link. */
  const peer = $derived(looksLikeKey ? q.replace(/^nostr:/i, '') : kind === 'link' ? (link?.pubkey ?? null) : null);
  /** The secret that goes with the request: the link's, else the typed code. */
  const linkInvite = $derived(kind === 'link' ? (link?.invite ?? null) : null);
  const typedCode = $derived(codeOpen ? normalizeCode(code) : null);
  /** The server of a pasted link (`srv=`): the chat is the private one there, after «Добавить сервер?». */
  const privateServer = $derived(kind === 'link' ? (link?.server ?? null) : null);
  /** The request goes with a secret and maybe a message: one «Отправить» instead of opening the chat. */
  const sending = $derived(!!peer && !privateServer && (!!linkInvite || codeOpen));

  // A pasted link is read by the runtime; the answer counts only for the text it was asked about.
  let asked = '';
  $effect(() => {
    const wanted = q;
    if (kind !== 'link') { link = null; linkError = ''; asked = ''; return; }
    if (wanted === asked) return;
    asked = wanted;
    link = null;
    linkError = '';
    messengerApi.links.inspect([wanted as InternalLinkText]).then(([view]) => {
      if (asked !== wanted) return;
      const c = invitedContact(view);
      if (!c) linkError = $t('msg_inv_err_not_contact');
      else if (c.isMe) linkError = $t('msg_inv_err_is_me');
      else if (c.blocked) linkError = $t('msg_inv_err_blocked');
      else link = c;
    }).catch((e) => { if (asked === wanted) linkError = messengerError(e); });
  });

  function reset() {
    key = ''; code = ''; text = ''; codeOpen = false; link = null; linkError = ''; asked = '';
  }

  async function start(p: string) {
    error = ''; busy = true;
    try {
      await chatStore.openPeer(p);
      open = false; reset();
      onopened?.();
    } catch (e) { error = messengerError(e); }
    finally { busy = false; }
  }

  /** The request with the secret (and with a code, the first message), then the chat it opened. */
  async function send() {
    if (!peer || busy) return;
    error = '';
    const invite = linkInvite ?? typedCode;
    if (!invite) { error = $t('msg_inv_code_bad'); return; }
    // The code goes with the first message: it is what the person reads as my first words.
    if (!linkInvite && !text.trim()) { error = $t('msg_inv_code_needs_text'); return; }
    busy = true;
    try {
      // The chat is opened first: it turns a typed npub into the key the request names.
      const chat = await messengerApi.chats.open(peer);
      await inviteStore.request(chat.peer_pubkey ?? peer, invite, text);
      await chatStore.openPeer(chat.peer_pubkey ?? peer);
      open = false; reset();
      onopened?.();
    } catch (e) {
      // The other side takes a first text plain and short only; the runtime refuses what it would drop.
      error = dmErrorCode(e) === 'dm_request_text' ? $t('msg_inv_err_request_text') : messengerError(e);
    } finally { busy = false; }
  }

  /**
   * The private chat on the server of the link: the server is added after
   * the question (nothing happens on «Отмена»), then the chat opens there,
   * approved on my side and told at once, with the link's secret.
   */
  async function openPrivate() {
    if (!link || !privateServer || busy) return;
    error = ''; busy = true;
    try {
      const id = await acceptServer(privateServer);
      if (!id) return;
      const chat = await messengerApi.servers.openPrivate(link.pubkey, id, link.invite);
      await chatStore.openPeer(chat.id);
      open = false; reset();
      onopened?.();
    } catch (e) { error = messengerError(e); }
    finally { busy = false; }
  }

  const pick = (c: MessengerContact) => start(c.pubkey);

  /** A key starts its chat; a name does when it leaves one contact; with a secret, the request goes. */
  function enter() {
    if (busy) return;
    if (privateServer) openPrivate();
    else if (sending) send();
    else if (peer) start(peer);
    else if (contacts.length === 1) pick(contacts[0]);
  }
</script>

<Dialog bind:open title={$t('msg_newchat_title')} width="min(440px, calc(100vw - 24px))">
  <div class="body">
    <input type="text" bind:value={key} placeholder={$t('msg_newchat_placeholder')} aria-label={$t('msg_newchat_placeholder')} spellcheck="false" disabled={busy}
      onkeydown={(e) => { if (e.key === 'Enter' && !e.isComposing) enter(); }} />

    {#if shortcuts && !q}
      <ul class="list shortcuts">
        <li><button class="row" onclick={() => go(contactsHref())}><span class="sc"><Icon name="book-user" size={18} /></span><span class="name">{$t('msg_contacts_title')}</span><Icon name="chevron-right" size={16} /></button></li>
        <li><button class="row" onclick={() => go(qrHref(true))}><span class="sc"><Icon name="scan" size={18} /></span><span class="name">{$t('msg_newchat_scan')}</span><Icon name="chevron-right" size={16} /></button></li>
        <li><button class="row" onclick={() => go(qrHref())}><span class="sc"><Icon name="qr-code" size={18} /></span><span class="name">{$t('msg_newchat_my_qr')}</span><Icon name="chevron-right" size={16} /></button></li>
      </ul>
    {/if}
    {#if !q && serverStore.list?.length}
      <!-- Someone I never talked to: my link of a private chat, to send by any way. -->
      <ul class="list shortcuts">
        <li><button class="row" onclick={() => { open = false; privateInvite.show(); }}><span class="sc"><Icon name="server-2" size={18} /></span><span class="name">{$t('msg_srv_invite_title')}</span><Icon name="chevron-right" size={16} /></button></li>
      </ul>
    {/if}

    {#if linkError}<div class="error-msg">{linkError}</div>{/if}
    {#if link}
      <div class="person">
        <Avatar url={null} label={link.name || '?'} seed={link.pubkey} size={34} />
        <span class="person-text">
          <span class="name">{link.name || `${link.pubkey.slice(0, 12)}…`}</span>
          {#if link.invite}<span class="note"><Icon name="check-circle" size={12} />{$t('msg_inv_link_has_invite')}</span>{/if}
          {#if privateServer}<span class="server-note"><Icon name="message-circle" size={12} />{$t('msg_srv_card_private_chat')}</span>{/if}
        </span>
      </div>
      {#if privateServer}<ServerBox server={privateServer} />{/if}
    {/if}

    {#if privateServer}
      <button class="btn btn-primary" disabled={busy} onclick={openPrivate}>
        <Icon name="message-circle" size={13} />{$t('msg_srv_card_open_private')}
      </button>
    {:else if peer && !linkInvite}
      {#if codeOpen}
        <input class="code mono" type="text" inputmode="numeric" autocomplete="one-time-code" bind:value={code} placeholder={$t('msg_inv_code_placeholder')}
          disabled={busy} aria-label={$t('msg_inv_code_placeholder')} />
      {:else}
        <button class="more" type="button" onclick={() => (codeOpen = true)}><Icon name="key" size={13} />{$t('msg_inv_have_code')}</button>
      {/if}
    {/if}

    {#if sending}
      <textarea rows="3" bind:value={text} placeholder={$t('msg_inv_message_placeholder')} aria-label={$t('msg_inv_message_placeholder')} disabled={busy} maxlength="1000"></textarea>
      <button class="btn btn-primary" disabled={busy || (codeOpen && !linkInvite && (!typedCode || !text.trim()))} onclick={send}>
        <Icon name="send" size={13} />{$t('msg_inv_send')}
      </button>
    {:else if peer && !privateServer}
      <button class="btn btn-primary" disabled={busy} onclick={() => peer && start(peer)}>{$t('msg_newchat_start')}</button>
    {/if}
    {#if error}<div class="error-msg">{error}</div>{/if}

    {#if contacts.length}
      <div class="label">{$t('msg_contacts_title')}</div>
      <ul class="list">
        {#each contacts as c (c.pubkey)}
          <li>
            <button class="row" disabled={busy} onclick={() => pick(c)}>
              <Avatar url={c.profile?.picture ?? null} label={contactLabel(c)} seed={c.pubkey} size={34} />
              <span class="name">{contactLabel(c)}</span>
              <code>{c.npub.slice(0, 12)}…</code>
            </button>
          </li>
        {/each}
      </ul>
    {:else if !q}
      <p class="hint">{$t('msg_newchat_hint')}</p>
    {:else if !kind}
      <p class="hint">{$t('msg_chats_nothing_found')}</p>
    {/if}
  </div>
</Dialog>

<style>
  .body { display: flex; flex-direction: column; gap: var(--sp-3); }
  input, textarea {
    font: inherit; font-size: var(--fs-sm); color: var(--text); width: 100%;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-field); padding: 9px 12px;
  }
  textarea { resize: vertical; min-height: 64px; line-height: 1.4; }
  input:focus, textarea:focus { outline: none; border-color: var(--accent-border); }
  .code { letter-spacing: 2px; font-variant-numeric: tabular-nums; }
  .more {
    display: inline-flex; align-items: center; gap: 6px; align-self: flex-start; padding: 0; border: 0; background: none; cursor: pointer;
    font: inherit; font-size: var(--fs-xs); color: var(--accent-text-3); text-align: left; line-height: 1.4;
  }
  .more :global(svg) { flex-shrink: 0; }
  .person { display: flex; align-items: center; gap: var(--sp-3); min-width: 0; }
  .person-text { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .note { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-xs); color: var(--success-text); }
  .server-note { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-xs); color: var(--text-2); }
  .btn :global(svg) { margin-right: 2px; }
  .label { font-size: var(--fs-2xs); text-transform: uppercase; letter-spacing: 0.6px; color: var(--text-3); font-weight: var(--fw-bold); }
  .list { list-style: none; margin: 0; padding: 0; max-height: 300px; overflow-y: auto; display: flex; flex-direction: column; gap: 2px; }
  .row {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; padding: 6px 8px; text-align: left;
    border: none; border-radius: var(--radius-sm); background: none; color: inherit; font: inherit; cursor: pointer;
  }
  .row:hover:not(:disabled) { background: var(--surface-row-hover); }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-semibold); flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .shortcuts { max-height: none; }
  .shortcuts .row { min-height: 48px; color: var(--text-2); }
  .shortcuts .name { color: var(--text); }
  .sc { width: 34px; height: 34px; border-radius: 50%; flex-shrink: 0; display: inline-flex; align-items: center; justify-content: center; background: var(--accent-tint); color: var(--accent-text-2); }
  code { font-family: var(--font-mono); font-size: var(--fs-2xs); color: var(--text-3); }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.5; }
  @media (pointer: coarse) {
    .more { min-height: 40px; }
  }
</style>
