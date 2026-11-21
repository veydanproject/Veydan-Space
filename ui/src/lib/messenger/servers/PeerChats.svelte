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
  import Tile from '$lib/core/ui/Tile.svelte';
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
    <ul class="list-rows rows">
      {#each [...(home ? [home] : []), ...privates] as c (c.id)}
        <li>
          <button type="button" class="list-row dense" class:here={c.id === current} disabled={c.id === current} onclick={() => onopen(c.id)}>
            <Tile icon={c.server_id ? 'server-2' : 'message-circle'} />
            <span class="list-row-body">
              <span class="top">
                {#if c.server_id}<ServerLabel server={c.server_id} size="md" />{:else}<span class="home">{$t('msg_srv_peer_home')}</span>{/if}
                {#if c.unread > 0}<span class="count-pill lg">{c.unread > 99 ? '99+' : c.unread}</span>{/if}
                <span class="stamp">{c.id === current ? $t('msg_srv_peer_here') : listStamp(c.last_message_at)}</span>
              </span>
              <span class="list-row-line">{line(c)}</span>
            </span>
          </button>
        </li>
      {/each}
      {#each missing as s (s.id)}
        <li>
          <button type="button" class="list-row dense" onclick={() => privateInvite.show({ peer: pubkey, server: s.id })}>
            <Tile icon="user-plus" />
            <span class="list-row-body"><span class="open-text">{$t('msg_srv_peer_open', { name: nameOfServer(s) })}</span></span>
          </button>
        </li>
      {/each}
    </ul>
  </section>
{/if}

<style>
  .peer-chats { display: flex; flex-direction: column; gap: var(--sp-2); padding: var(--sp-3) var(--sp-4); border-radius: var(--radius-lg); background: var(--surface-2); border: 1px solid var(--border); }
  /* The title of the card in sentence case, as the other cards of the page. */
  h3 { margin: 0; font-size: var(--fs-base); font-weight: var(--fw-bold); color: var(--text); }
  /* The dense rows of base.css, the tile in place of the avatar; the row's hover reaches near the card's edge. */
  .rows { margin: 0 calc(-1 * var(--sp-3)); padding: 0; }
  .list-row :global(svg) { flex-shrink: 0; }
  .list-row.here { opacity: 0.75; }
  .top { display: flex; align-items: center; gap: var(--sp-1-5); min-width: 0; }
  .home { font-size: var(--fs-base); font-weight: var(--fw-bold); color: var(--text); }
  .stamp { margin-left: auto; font-size: var(--fs-xs); color: var(--text-3); flex-shrink: 0; font-variant-numeric: tabular-nums; }
  /* The unread count is the .count-pill of base.css, 22 on a row. */
  .open-text { font-size: var(--fs-base); color: var(--text); font-weight: var(--fw-bold); line-height: var(--lh-snug); overflow-wrap: anywhere; }
</style>
