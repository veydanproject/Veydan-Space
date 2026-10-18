<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A server someone shared (internal/messenger-wire.md, «Принятие сервера»):
  its card alone (`veydan://server/…`) — «Добавить сервер»; a person's link
  with the card of the server they are on — «Открыть частный чат» there; a
  group's link with the card of its server — «Вступить» there (GroupCard
  draws a group, with the server under it). What the server is, its relay
  host, what it has, how phones are woken and whether it is Veydan's are
  shown before anything is pressed, and «Добавить сервер?» asks again,
  naming all of it, before the server is added.
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Avatar from '../../contacts/Avatar.svelte';
  import { messengerError, type LinkView, type ServerPreview } from '../../api';
  import { acceptServer, hostOf, serverFacts, serverName } from '../../servers/serverAccept.svelte';
  import { linkActions } from '../actions';
  import { linkStore } from '../linkStore.svelte';

  type ServerView = Extract<LinkView, { kind: 'server' }>;
  type ContactView = Extract<LinkView, { kind: 'contact' }>;

  interface Props { view: ServerView | ContactView }
  let { view }: Props = $props();

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'msg_you', params);

  const server = $derived<ServerPreview | null>(view.kind === 'server' ? view : view.server);
  const person = $derived(view.kind === 'contact' ? view : null);
  /** Added from this card. */
  let added = $state(false);
  let busy = $state(false);
  let error = $state('');

  const known = $derived(added || !!server?.known);

  async function run(fn: () => Promise<unknown>) {
    error = ''; busy = true;
    try { await fn(); }
    catch (e) { error = messengerError(e); }
    finally { busy = false; }
  }

  function add() {
    if (!server) return;
    const s = server;
    run(async () => {
      if (await acceptServer(s)) {
        added = true;
        linkStore.refresh();
      }
    });
  }

  function openPrivate() {
    if (!person) return;
    const p = person;
    run(async () => {
      if (await linkActions.writePrivate(p)) added = true;
    });
  }
</script>

{#if server}
  <div class="box">
    {#if person}
      <div class="top">
        <Avatar url={person.picture} label={person.name || person.npub} seed={person.pubkey} size={44} />
        <div class="about">
          <span class="name">{person.name.trim() || `${person.npub.slice(0, 12)}…`}</span>
          <span class="sub">{$t('msg_srv_card_private_chat', { name: serverName(server, tr) })}</span>
        </div>
      </div>
    {:else}
      <div class="top">
        <Icon name={server.is_veydan ? 'shield-check' : 'server-2'} size={22} />
        <div class="about">
          <span class="name">{serverName(server, tr)}</span>
          <span class="sub"><code>{hostOf(server.relay)}</code></span>
        </div>
      </div>
    {/if}
    <div class="facts">
      <span class="whose" class:veydan={server.is_veydan}>
        <Icon name={server.is_veydan ? 'shield-check' : 'info'} size={12} />{$t(server.is_veydan ? 'msg_srv_is_veydan' : 'msg_srv_not_veydan')}
      </span>
      <span>{serverFacts(server, tr).join(' · ')}</span>
    </div>
    {#if error}<div class="error">{error}</div>{/if}
    <div class="row">
      {#if person}
        {#if person.is_me}
          <span class="known"><Icon name="user" size={12} />{$t('msg_card_contact_me')}</span>
        {:else if person.blocked}
          <span class="known"><Icon name="user-cancel" size={12} />{$t('msg_card_contact_blocked')}</span>
        {:else}
          <button class="btn btn-primary btn-sm" disabled={busy} onclick={openPrivate}>
            <Icon name="message-circle" size={13} />{$t('msg_srv_card_open_private')}
          </button>
        {/if}
      {:else if known}
        <span class="known"><Icon name="check" size={12} />{$t('msg_srv_card_known')}</span>
      {:else}
        <button class="btn btn-primary btn-sm" disabled={busy} onclick={add}>
          <Icon name="plus" size={13} />{$t('msg_srv_card_add')}
        </button>
      {/if}
    </div>
  </div>
{/if}

<style>
  .box { display: flex; flex-direction: column; gap: var(--sp-2); }
  .top { display: flex; align-items: center; gap: var(--sp-3); min-width: 0; color: var(--text-2); }
  .about { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-bold); color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { font-size: var(--fs-2xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub code { font-family: var(--font-mono); }
  .facts { display: flex; flex-direction: column; gap: 2px; font-size: var(--fs-xs); color: var(--text-2); line-height: 1.4; overflow-wrap: anywhere; }
  .whose { display: inline-flex; align-items: center; gap: 4px; color: var(--warn-text); }
  .whose.veydan { color: var(--success-text); }
  .row { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .known { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-2xs); color: var(--text-3); padding: 0 4px; }
  .error { font-size: var(--fs-xs); color: var(--danger-text); line-height: 1.4; }
</style>
