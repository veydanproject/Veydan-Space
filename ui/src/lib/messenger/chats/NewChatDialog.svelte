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
  import { addressKind, invitedContact, normalizeCode, webLoginHref, type InvitedContact } from '../invites/invites';
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
  /** A pasted link of a sign-in to the web cabinet: its id, offered to the screen of approval. */
  let webId = $state<string | null>(null);

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
    if (kind !== 'link') { link = null; linkError = ''; webId = null; asked = ''; return; }
    if (wanted === asked) return;
    asked = wanted;
    link = null;
    linkError = '';
    webId = null;
    messengerApi.links.inspect([wanted as InternalLinkText]).then(([view]) => {
      if (asked !== wanted) return;
      if (view?.kind === 'web') { webId = view.id; return; }
      const c = invitedContact(view);
      if (!c) linkError = $t('msg_inv_err_not_contact');
      else if (c.isMe) linkError = $t('msg_inv_err_is_me');
      else if (c.blocked) linkError = $t('msg_inv_err_blocked');
      else link = c;
    }).catch((e) => { if (asked === wanted) linkError = messengerError(e); });
  });

  function reset() {
    key = ''; code = ''; text = ''; codeOpen = false; link = null; linkError = ''; webId = null; asked = '';
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

<!-- The window's verb (open the sign-in, the private chat, send, start) stays at its foot while the contacts scroll (the owner's rule). -->
{#snippet verbs()}
  {#if webId}
    {@const id = webId}
    <button class="btn btn-primary" onclick={() => { reset(); go(webLoginHref(id)); }}><Icon name="world" size={18} />{$t('msg_inv_web_open')}</button>
  {/if}
  {#if privateServer}
    <button class="btn btn-primary" disabled={busy} onclick={openPrivate}>
      <Icon name="message-circle" size={18} />{$t('msg_srv_card_open_private')}
    </button>
  {/if}
  {#if sending}
    <button class="btn btn-primary" disabled={busy || (codeOpen && !linkInvite && (!typedCode || !text.trim()))} onclick={send}>
      <Icon name="send" size={18} />{$t('msg_inv_send')}
    </button>
  {:else if peer && !privateServer}
    <button class="btn btn-primary" disabled={busy} onclick={() => peer && start(peer)}>{$t('msg_newchat_start')}</button>
  {/if}
{/snippet}

<Dialog bind:open title={$t('msg_newchat_title')} width="min(440px, calc(100vw - 24px))"
  footer={webId || privateServer || sending || (peer && !privateServer) ? verbs : undefined}>
  <div class="body">
    <label class="field">
      <Icon name="search" size={16} />
      <input type="text" bind:value={key} placeholder={$t('msg_newchat_placeholder')} aria-label={$t('msg_newchat_placeholder')} spellcheck="false" disabled={busy}
        onkeydown={(e) => { if (e.key === 'Enter' && !e.isComposing) enter(); }} />
    </label>

    {#if !q && (shortcuts || serverStore.list?.length)}
      <!-- The ways to start a chat: tiles in one row, the icon over the word (a phone also reaches the contacts and the QR codes here); one alone is a wide tile. -->
      <ul class="ways">
        {#if shortcuts}
          <li><button class="way" onclick={() => go(contactsHref())}><Icon name="book-user" size={18} /><span>{$t('msg_contacts_title')}</span></button></li>
          <li><button class="way" onclick={() => go(qrHref(true))}><Icon name="scan" size={18} /><span>{$t('msg_newchat_scan')}</span></button></li>
          <li><button class="way" onclick={() => go(qrHref())}><Icon name="qr-code" size={18} /><span>{$t('msg_newchat_my_qr')}</span></button></li>
        {/if}
        {#if serverStore.list?.length}
          <!-- Someone I never talked to: my link of a private chat, to send by any way. -->
          <li><button class="way" onclick={() => { open = false; privateInvite.show(); }}><Icon name="server-2" size={18} /><span>{$t('msg_srv_invite_title')}</span></button></li>
        {/if}
      </ul>
    {/if}

    {#if linkError}<div class="error-msg">{linkError}</div>{/if}
    {#if webId}
      <!-- Pasted here by me: the screen of approval asks before anything is approved. -->
      <p class="hint">{$t('msg_inv_web_link')}</p>
    {/if}
    {#if link}
      <div class="person">
        <Avatar url={null} label={link.name || '?'} seed={link.pubkey} size={36} />
        <span class="person-text">
          <span class="name">{link.name || `${link.pubkey.slice(0, 12)}…`}</span>
          {#if link.invite}<span class="note"><Icon name="check-circle" size={12} />{$t('msg_inv_link_has_invite')}</span>{/if}
          {#if privateServer}<span class="server-note"><Icon name="message-circle" size={12} />{$t('msg_srv_card_private_chat')}</span>{/if}
        </span>
      </div>
      {#if privateServer}<ServerBox server={privateServer} />{/if}
    {/if}

    {#if !privateServer && peer && !linkInvite}
      {#if codeOpen}
        <input class="code mono" type="text" inputmode="numeric" autocomplete="one-time-code" bind:value={code} placeholder={$t('msg_inv_code_placeholder')}
          disabled={busy} aria-label={$t('msg_inv_code_placeholder')} />
      {:else}
        <button class="more" type="button" onclick={() => (codeOpen = true)}><Icon name="key" size={13} />{$t('msg_inv_have_code')}</button>
      {/if}
    {/if}

    {#if sending}
      <textarea rows="3" bind:value={text} placeholder={$t('msg_inv_message_placeholder')} aria-label={$t('msg_inv_message_placeholder')} disabled={busy} maxlength="1000"></textarea>
    {/if}
    {#if error}<div class="error-msg">{error}</div>{/if}

    {#if contacts.length}
      <div class="label">{$t('msg_contacts_title')}</div>
      <ul class="list">
        {#each contacts as c (c.pubkey)}
          <li>
            <button class="row" disabled={busy} onclick={() => pick(c)}>
              <Avatar url={c.profile?.picture ?? null} label={contactLabel(c)} seed={c.pubkey} size={36} />
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
  /* The field of whom: 44 high with the search glyph (48 under a finger). */
  .field {
    display: flex; align-items: center; gap: var(--sp-2); min-height: var(--control-h-lg); padding: 0 var(--sp-3);
    background: var(--surface-3); border: 1px solid var(--border); border-radius: var(--radius); color: var(--text-3);
  }
  .field:focus-within { border-color: var(--accent-border); box-shadow: 0 0 0 3px var(--accent-bg); }
  .field :global(svg) { flex-shrink: 0; }
  .field input { flex: 1; min-width: 0; align-self: stretch; padding: 0; border: 0; background: none; font: inherit; font-size: var(--fs-base); color: var(--text); }
  .field input:focus { outline: none; box-shadow: none; }
  .code, textarea {
    font: inherit; font-size: var(--fs-base); color: var(--text); width: 100%;
    background: var(--surface-3); border: 1px solid var(--border); border-radius: var(--radius); padding: var(--sp-2) var(--sp-3);
  }
  .code { min-height: var(--control-h-lg); letter-spacing: 2px; font-variant-numeric: tabular-nums; }
  textarea { resize: vertical; min-height: 64px; line-height: 1.4; }
  .code:focus, textarea:focus { outline: none; border-color: var(--accent-border); }
  .more {
    display: inline-flex; align-items: center; gap: 6px; align-self: flex-start; padding: 0; border: 0; background: none; cursor: pointer;
    font: inherit; font-size: var(--fs-xs); color: var(--accent-text-3); text-align: left; line-height: 1.4;
  }
  .more :global(svg) { flex-shrink: 0; }
  .more:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; border-radius: var(--radius-xs); }
  .person { display: flex; align-items: center; gap: var(--sp-3); min-width: 0; }
  .person-text { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .note { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-xs); color: var(--success-text); }
  .server-note { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-xs); color: var(--text-2); }
  /* The ways to start a chat: tiles of one row, the icon in the accent over the word. */
  .ways { list-style: none; margin: 0; padding: 0; display: grid; grid-auto-flow: column; grid-auto-columns: minmax(0, 1fr); gap: var(--sp-2); }
  .ways li { display: flex; }
  .way {
    display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 6px; width: 100%; height: 100%; min-height: 76px;
    padding: var(--sp-2) 0; border: 1px solid var(--border); border-radius: var(--radius-md); background: var(--surface-2);
    color: var(--text); font: inherit; font-size: var(--fs-xs); font-weight: var(--fw-semibold); line-height: 1.25; text-align: center; cursor: pointer; hyphens: auto;
  }
  .way :global(svg) { flex-shrink: 0; color: var(--accent-text); }
  .way span { overflow-wrap: break-word; text-wrap: balance; }
  .way:hover { background: var(--surface-row-hover); }
  .way:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  /* One way alone (a computer: «Пригласить в частный чат»): a wide tile 48, the icon before the word. */
  .ways li:only-child .way { flex-direction: row; justify-content: flex-start; gap: var(--sp-3); min-height: 48px; padding: 0 var(--sp-3); font-size: var(--fs-base); text-align: left; }
  /* «Контакты» in sentence case over their rows. */
  .label { margin-top: var(--sp-1); font-size: var(--fs-sm); font-weight: var(--fw-bold); color: var(--text-2); }
  /* The contacts: rows right under the ways, no card around them (the row's hover reaches the dialog's padding). */
  .list { list-style: none; margin: 0 calc(-1 * var(--sp-2)); padding: 0; display: flex; flex-direction: column; }
  .row {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; min-height: 48px; padding: var(--sp-1) var(--sp-2); text-align: left;
    border: none; border-radius: var(--radius-md); background: none; color: var(--text-3); font: inherit; cursor: pointer;
  }
  .row:hover:not(:disabled) { background: var(--surface-row-hover); }
  .row:focus-visible { outline: 2px solid var(--accent-border); outline-offset: -2px; }
  .name { flex: 1; min-width: 0; font-size: var(--fs-base); font-weight: var(--fw-bold); color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  code { flex-shrink: 0; font-size: var(--fs-xs); color: var(--text-3); }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.5; }
  /* A narrow screen: the tiles in a grid of 2×2, each as high as its row's tallest. */
  @media (max-width: 520px) {
    .ways { grid-auto-flow: row; grid-template-columns: repeat(2, minmax(0, 1fr)); grid-auto-columns: auto; }
    .ways li:only-child { grid-column: 1 / -1; }
  }
  @media (pointer: coarse) {
    .field { min-height: var(--sp-12); }
    .row { min-height: 56px; }
    .way { min-height: 80px; }
    .more { min-height: 40px; }
  }
</style>
