<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «Пригласить в частный чат» (privateInvite.show): which server of mine
  (ServerBox says what it is), then the ways in — to the person it was
  opened for, «Отправить приглашение» (my card with the server in our
  chat); to anyone, «Скопировать ссылку» (my link of the private chat,
  with an invitation: 5 people, 30 days) or «Отправить в чат». Mount once
  per page, beside ServerInfoDialog.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { toast } from '$lib/core/ui/toast.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import { nameStore } from '../groups/names.svelte';
  import { messengerError } from '../api';
  import ShareDialog from '../content/ShareDialog.svelte';
  import ServerBox from './ServerBox.svelte';
  import ServerPicker from './ServerPicker.svelte';
  import { serverStore } from './serverStore.svelte';
  import { invitePeer, privateInvite, privateLink } from './privateInvite.svelte';

  const ask = $derived(privateInvite.current);
  const peer = $derived(ask?.peer ?? null);
  let server = $state<string | null>(null);
  let busy = $state(false);
  let error = $state('');
  let sharing = $state(false);

  $effect(() => {
    if (!ask) return;
    error = '';
    if (serverStore.list === null) void serverStore.load();
    const list = serverStore.list ?? [];
    if (!list.some((s) => s.id === server)) server = ask.server && list.some((s) => s.id === ask.server) ? ask.server : (list[0]?.id ?? null);
  });

  const chosen = $derived(serverStore.byId(server));

  async function run(fn: () => Promise<unknown>) {
    error = ''; busy = true;
    try { await fn(); }
    catch (e) { error = messengerError(e); }
    finally { busy = false; }
  }

  function send() {
    if (!peer || !server) return;
    const p = peer, s = server;
    void run(async () => {
      await invitePeer(p, s);
      privateInvite.close();
      toast.success($t('msg_srv_invite_sent'));
    });
  }

  function copy() {
    if (!server) return;
    const s = server;
    void run(async () => {
      await navigator.clipboard.writeText(await privateLink(s));
      toast.success($t('msg_srv_invite_copied'));
    });
  }

  function share() {
    privateInvite.close();
    sharing = true;
  }
</script>

<Dialog open={ask !== null} title={$t('msg_srv_invite_title')} width="min(420px, calc(100vw - 24px))" onclose={() => privateInvite.close()}>
  {#if ask}
    <div class="body">
      {#if peer}
        <div class="who">
          <Avatar label={nameStore.label(peer)} seed={peer} size={36} url={null} />
          <span class="name">{nameStore.label(peer)}</span>
        </div>
      {/if}
      <p class="lead">{$t(peer ? 'msg_srv_invite_lead_peer' : 'msg_srv_invite_lead')}</p>
      {#if (serverStore.list?.length ?? 0) > 1}<ServerPicker bind:value={server} disabled={busy} />{/if}
      {#if chosen}
        <ServerBox server={{ ...chosen, is_veydan: false }} />
      {:else if serverStore.list}
        <p class="lead">{$t('msg_srv_list_empty')}</p>
      {/if}
      {#if error}<div class="error-msg">{error}</div>{/if}
      <div class="ways">
        {#if peer}
          <button type="button" class="btn btn-primary" disabled={busy || !chosen} onclick={send}>
            <Icon name={busy ? 'loader' : 'send'} size={14} />{$t('msg_srv_invite_send')}
          </button>
        {/if}
        <button type="button" class="btn" class:btn-primary={!peer} class:btn-ghost={!!peer} disabled={busy || !chosen} onclick={copy}>
          <Icon name="copy" size={14} />{$t('msg_srv_invite_copy')}
        </button>
        {#if !peer}
          <button type="button" class="btn btn-ghost" disabled={busy || !chosen} onclick={share}>
            <Icon name="message-circle" size={14} />{$t('msg_srv_invite_share')}
          </button>
        {/if}
      </div>
      <p class="note">{$t('msg_srv_invite_link_hint')}</p>
    </div>
  {/if}
</Dialog>

<ShareDialog bind:open={sharing} card={{ pubkey: null }} {server} />

<style>
  .body { display: flex; flex-direction: column; gap: var(--sp-3); }
  .who { display: flex; align-items: center; gap: var(--sp-2); min-width: 0; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .lead { margin: 0; font-size: var(--fs-sm); color: var(--text-body); line-height: 1.5; }
  .ways { display: flex; flex-direction: column; gap: var(--sp-2); }
  .ways .btn { justify-content: center; }
  .note { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.45; }
</style>
