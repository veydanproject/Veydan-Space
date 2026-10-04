<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Send a link into a chat. What goes out is an ordinary message with the
  link in it; the other side draws the card from the link.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import MessageContent from './MessageContent.svelte';
  import { chatStore } from '../chats/chatStore.svelte';
  import { dmErrorCode, messengerError, type MessengerChat } from '../api';

  interface Props {
    open: boolean;
    /** The link to send, or how to get it. */
    link: string | (() => Promise<string>);
    /** A chat the link makes no sense in (the group itself). */
    exclude?: string | null;
  }
  let { open = $bindable(), link, exclude = null }: Props = $props();

  let query = $state('');
  let text = $state('');
  let busy = $state(false);
  let error = $state('');
  let sent = $state<string[]>([]);

  let wasOpen = false;
  $effect(() => {
    if (open && !wasOpen) {
      query = ''; error = ''; sent = []; text = typeof link === 'string' ? link : '';
      if (typeof link !== 'string') link().then((l) => (text = l)).catch((e) => (error = messengerError(e)));
      chatStore.loadChats().catch(() => {});
    }
    wasOpen = open;
  });

  const q = $derived(query.trim().toLowerCase());
  const chats = $derived(
    chatStore.chats.filter((c) => c.can_send && !c.archived && c.id !== exclude && (!q || c.title.toLowerCase().includes(q))),
  );

  async function send(chat: MessengerChat) {
    if (!text) return;
    error = ''; busy = true;
    try {
      await chatStore.sendTo(chat, text);
      sent = [...sent, chat.id];
    } catch (e) {
      const code = dmErrorCode(e);
      error = code ? $t(`msg_err_${code}` as 'msg_err_dm_blocked', { name: chat.title }) : messengerError(e);
    } finally { busy = false; }
  }
</script>

<Dialog bind:open title={$t('msg_share_title')} width="min(440px, calc(100vw - 24px))">
  <div class="body">
    {#if text}<div class="what"><MessageContent {text} /></div>{/if}
    <input type="text" bind:value={query} placeholder={$t('msg_share_search')} spellcheck="false" />
    {#if error}<div class="error-msg">{error}</div>{/if}
    {#if chats.length}
      <ul class="list">
        {#each chats as c (c.id)}
          {@const done = sent.includes(c.id)}
          <li>
            <button class="row" disabled={busy || done || !text} onclick={() => send(c)}>
              <Avatar url={c.picture} label={c.title} seed={c.peer_pubkey ?? c.id} size={34} />
              <span class="name">{c.title}</span>
              {#if c.kind === 'group'}<span class="dim"><Icon name="users" size={12} /></span>{/if}
              <span class="state" class:done><Icon name={done ? 'check' : 'send'} size={13} />{done ? $t('msg_share_sent') : ''}</span>
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
  input {
    font: inherit; font-size: var(--fs-sm); color: var(--text); width: 100%;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-field); padding: 9px 12px;
  }
  input:focus { outline: none; border-color: var(--accent-border); }
  .list { list-style: none; margin: 0; padding: 0; max-height: 300px; overflow-y: auto; display: flex; flex-direction: column; gap: 2px; }
  .row {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; padding: 6px 8px; text-align: left;
    border: none; border-radius: var(--radius-sm); background: none; color: inherit; font: inherit; cursor: pointer;
  }
  .row:hover:not(:disabled) { background: var(--surface-row-hover); }
  .row:disabled { cursor: default; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-semibold); flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dim { color: var(--text-3); display: inline-flex; }
  .state { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-2xs); color: var(--accent-text-2); flex-shrink: 0; }
  .state.done { color: var(--success-text, var(--success)); }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.5; }
  @media (pointer: coarse) { input { font-size: 16px; } .row { padding: 9px 8px; } }
</style>
