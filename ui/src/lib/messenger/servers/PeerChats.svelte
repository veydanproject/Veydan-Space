<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Every chat with one person (internal/messenger-wire.md, «Частный чат»):
  the home chat and each private one («Частный · S1»), each opening its
  chat, and «Пригласить в частный чат на …» for each private server of
  mine where there is no chat with them yet: it opens
  PrivateInviteDialog, which sends them my card with the server (opening
  the chat here alone would leave them, who may not have the server, deaf
  to it). Shown on the page of a contact
  and in the panel about the person of a chat. Nothing is shown while I
  have no private server and only the home chat exists.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import type { MessengerChat } from '../api';
  import { chatStore } from '../chats/chatStore.svelte';
  import { listStamp } from '../shared/time';
  import ServerLabel from './ServerLabel.svelte';
  import { nameOfServer, serverStore } from './serverStore.svelte';
  import { privateInvite } from './privateInvite.svelte';

  interface Props {
    pubkey: string;
    /** The chat already on the screen: marked, not offered again. */
    current?: string | null;
    /** Shows a chat (the page decides how: in place, or its own screen). */
    onopen: (chatId: string) => void;
  }
  let { pubkey, current = null, onopen }: Props = $props();

  onMount(() => { if (!serverStore.list) void serverStore.load(); });

  const home = $derived(chatStore.chats.find((c) => c.id === `dm:${pubkey}`) ?? null);
  const privates = $derived(chatStore.chats.filter((c) => c.kind === 'dm' && c.id.startsWith(`dm:${pubkey}@`)));
  const missing = $derived((serverStore.list ?? []).filter((s) => !privates.some((c) => c.server_id === s.id || c.id.endsWith(`@${s.id}`))));
  const shown = $derived(privates.length > 0 || missing.length > 0);

  function line(c: MessengerChat): string {
    if (c.last_preview) return c.last_preview;
    return c.last_message_at ? '' : $t('msg_chat_no_messages');
  }
</script>

{#if shown}
  <section class="peer-chats">
    <h3>{$t('msg_srv_peer_chats')}</h3>
    <ul class="rows">
      {#each [...(home ? [home] : []), ...privates] as c (c.id)}
        <li>
          <button type="button" class="row" class:here={c.id === current} disabled={c.id === current} onclick={() => onopen(c.id)}>
            <Icon name={c.server_id ? 'server-2' : 'message-circle'} size={16} />
            <span class="what">
              <span class="top">
                {#if c.server_id}<ServerLabel server={c.server_id} size="md" />{:else}<span class="home">{$t('msg_srv_peer_home')}</span>{/if}
                {#if c.unread > 0}<span class="count">{c.unread > 99 ? '99+' : c.unread}</span>{/if}
                <span class="stamp">{c.id === current ? $t('msg_srv_peer_here') : listStamp(c.last_message_at)}</span>
              </span>
              <span class="preview">{line(c)}</span>
            </span>
          </button>
        </li>
      {/each}
      {#each missing as s (s.id)}
        <li>
          <button type="button" class="row open" onclick={() => privateInvite.show({ peer: pubkey, server: s.id })}>
            <Icon name="user-plus" size={16} />
            <span class="what"><span class="open-text">{$t('msg_srv_peer_open', { name: nameOfServer(s) })}</span></span>
          </button>
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  .peer-chats { display: flex; flex-direction: column; gap: var(--sp-2); padding: var(--sp-4); border-radius: var(--radius-lg); background: var(--surface-2); border: 1px solid var(--border); }
  h3 { margin: 0; font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--text-3); }
  .rows { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .rows li + li { border-top: 1px solid var(--divider, var(--border)); }
  .row {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; min-height: 48px; padding: var(--sp-2) var(--sp-1);
    border: none; background: none; color: var(--text-2); font: inherit; text-align: left; cursor: pointer; border-radius: var(--radius-sm);
  }
  .row :global(svg) { flex-shrink: 0; }
  @media (hover: hover) { .row:hover:not(:disabled) { background: var(--surface-hover); color: var(--text); } }
  .row:disabled { cursor: default; }
  .row.here { opacity: 0.75; }
  .what { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .top { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .home { font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text); }
  .stamp { margin-left: auto; font-size: var(--fs-2xs); color: var(--text-3); flex-shrink: 0; }
  .count {
    min-width: 18px; height: 18px; padding: 0 5px; border-radius: var(--radius-badge); flex-shrink: 0;
    background: var(--accent); color: #fff; font-size: var(--fs-2xs); font-weight: var(--fw-bold); display: inline-flex; align-items: center; justify-content: center;
  }
  .preview { font-size: var(--fs-xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .open-text { font-size: var(--fs-sm); color: var(--accent-text-2); font-weight: var(--fw-semibold); line-height: 1.35; overflow-wrap: anywhere; }
</style>
