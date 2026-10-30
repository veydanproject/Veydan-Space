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

<Dialog open={ask !== null} title={$t('msg_srv_invite_title')} width="min(420px, calc(100vw - 24px))" onclose={() => privateInvite.close()}
  footer={ask ? ways : undefined}>
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
        <div class="srv"><ServerBox server={{ ...chosen, is_veydan: false }} /></div>
      {:else if serverStore.list}
        <p class="lead">{$t('msg_srv_list_empty')}</p>
      {/if}
      {#if error}<div class="error-msg">{error}</div>{/if}
      <p class="note">{$t('msg_srv_invite_link_hint')}</p>
    </div>
  {/if}
</Dialog>

<!-- The ways in stay at the window's foot while the server's card scrolls (the owner's rule). -->
{#snippet ways()}
  <div class="ways">
    {#if peer}
      <button type="button" class="btn btn-primary" disabled={busy || !chosen} onclick={send}>
        <Icon name={busy ? 'loader' : 'send'} size={18} />{$t('msg_srv_invite_send')}
      </button>
    {/if}
    <button type="button" class="btn" class:btn-primary={!peer} class:btn-ghost={!!peer} disabled={busy || !chosen} onclick={copy}>
      <Icon name="copy" size={18} />{$t('msg_srv_invite_copy')}
    </button>
    {#if !peer}
      <button type="button" class="btn btn-ghost" disabled={busy || !chosen} onclick={share}>
        <Icon name="message-circle" size={18} />{$t('msg_srv_invite_share')}
      </button>
    {/if}
  </div>
{/snippet}

<ShareDialog bind:open={sharing} card={{ pubkey: null }} {server} />

<style>
  .body { display: flex; flex-direction: column; gap: var(--sp-3); }
  .who { display: flex; align-items: center; gap: var(--sp-2); min-width: 0; }
  .name { font-size: var(--fs-base); font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .lead { margin: 0; font-size: var(--fs-base); color: var(--text-body); line-height: 1.5; }
  /* The server as one card of rows, a line between them (design book, «Пригласить в частный чат»). */
  .srv :global(.srv-box) { gap: 0; padding: var(--sp-1) var(--sp-3); border: 1px solid var(--border); border-radius: var(--radius-lg); }
  .srv :global(.head), .srv :global(.row) { align-items: center; padding: var(--sp-2) 0; font-size: var(--fs-sm); }
  .srv :global(.rows) { gap: 0; font-size: var(--fs-sm); }
  .srv :global(.row) { border-top: 1px solid var(--border); }
  .srv :global(dt), .srv :global(.head .label) { color: var(--text-2); }
  .srv :global(dd) { color: var(--text); }
  /* The ways side by side where their words fit, else one under the other, the whole width. */
  .ways { flex: 1; min-width: 0; display: flex; flex-wrap: wrap; gap: var(--sp-2); }
  .ways .btn { flex: 1 1 auto; justify-content: center; }
  .note { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.5; }
</style>
