<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  My private servers, in the settings of the servers: each with its relay,
  what it has (media, calls, how phones are woken), how many chats and
  groups live on it here, and «Удалить», which takes it from every device
  of mine with the private chats on it (the question names how many). A
  server is added by its card: one in a chat, a link that carries it, or
  the link pasted in «Добавить по ссылке» here — always through «Добавить
  сервер?».
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import Notice from '$lib/core/ui/Notice.svelte';
  import { countKey, locale, t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { ask } from '$lib/core/ui/confirm.svelte';
  import { toast } from '$lib/core/ui/toast.svelte';
  import { messengerApi, messengerError, type PrivateServerView } from '../api';
  import { acceptServer, hostOf } from './serverAccept.svelte';
  import { nameOfServer, serverStore } from './serverStore.svelte';
  import { privateInvite } from './privateInvite.svelte';

  let busy = $state(false);
  let error = $state('');
  let link = $state('');
  let adding = $state(false);
  let addError = $state('');

  // «Ввести код»: the invitation code of a server that said no; the list is read again on `servers.updated` (serverStore).
  let joinFor = $state<PrivateServerView | null>(null);
  let code = $state('');
  let joining = $state(false);
  let joinError = $state('');

  const JOIN_ERRORS: Record<string, string> = {
    server_invite_invalid: 'msg_srv_invite_invalid',
    server_invite_used: 'msg_srv_invite_used',
    server_invite_expired: 'msg_srv_invite_expired',
  };

  function openJoin(s: PrivateServerView) { joinFor = s; code = ''; joinError = ''; }

  async function join() {
    const s = joinFor;
    if (!s || !code.trim()) return;
    joinError = ''; joining = true;
    try {
      await messengerApi.servers.join(s.id, code.trim());
      joinFor = null;
      await serverStore.load();
    } catch (e) {
      const hit = Object.keys(JOIN_ERRORS).find((k) => new RegExp(`\\b${k}\\b`).test(messengerError(e)));
      joinError = $t((hit ? JOIN_ERRORS[hit] : 'msg_srv_join_failed') as 'msg_you');
    } finally {
      joining = false;
    }
  }

  onMount(() => { void serverStore.load(); });

  const chatsText = (n: number) => $t(countKey('msg_srv_chats_count', n, $locale), { n: String(n) });
  const groupsText = (n: number) => $t(countKey('msg_srv_groups_count', n, $locale), { n: String(n) });

  async function remove(s: PrivateServerView) {
    const chats = s.chats ?? 0;
    const groups = s.groups ?? 0;
    const yes = await ask({
      title: $t('msg_srv_remove_title', { name: nameOfServer(s) }),
      message: chats || groups
        ? $t('msg_srv_remove_counts', { chats: chatsText(chats), groups: groupsText(groups) })
        : $t('msg_srv_remove_text'),
      confirmLabel: $t('msg_srv_remove'),
    });
    if (!yes) return;
    error = ''; busy = true;
    try { await serverStore.remove(s.id); }
    catch (e) { error = messengerError(e); }
    finally { busy = false; }
  }

  /** A pasted card (`veydan://server/…`, or a link of a person or a group that carries one): «Добавить сервер?» first. */
  async function addByLink() {
    const text = link.trim();
    if (!text) return;
    addError = ''; adding = true;
    try {
      const preview = await messengerApi.servers.preview(text);
      if (preview.known) {
        toast.info($t('msg_srv_add_known', { name: preview.name.trim() || hostOf(preview.relay) }));
        link = '';
        return;
      }
      if (await acceptServer(preview)) {
        link = '';
        await serverStore.load();
      }
    } catch (e) {
      addError = /\bserver_link_none\b/.test(messengerError(e)) ? $t('msg_srv_add_bad') : messengerError(e);
    } finally {
      adding = false;
    }
  }
</script>

<div class="private">
  <div class="title"><Icon name="server-2" size={16} />{$t('msg_srv_list_title')}</div>
  <p class="meta">{$t('msg_srv_list_hint')}</p>
  {#if serverStore.list && serverStore.list.length}
    <ul class="list">
      {#each serverStore.list as s (s.id)}
        <li class="row">
          <div class="info">
            <span class="name">{nameOfServer(s)}</span>
            <code>{hostOf(s.relay)}</code>
            <span class="facts">
              <span class="fact" class:off={!s.media}><Icon name="image" size={11} />{$t(s.media ? 'msg_srv_has_media' : 'msg_srv_media_none')}</span>
              <span class="fact" class:off={!s.call_node}><Icon name="phone" size={11} />{$t(s.call_node ? 'msg_srv_has_calls' : 'msg_srv_calls_none')}</span>
              <span class="fact" class:off={s.push !== 'veydan'}><Icon name="bell" size={11} />{$t(s.push === 'veydan' ? 'msg_srv_push_veydan' : 'msg_srv_push_off')}</span>
            </span>
            <span class="meta">{chatsText(s.chats ?? 0)} · {groupsText(s.groups ?? 0)}</span>
            {#if s.denied}
              <span class="denied" title={s.denied}>
                <Icon name="lock" size={12} />{$t('msg_srv_no_access')}
                <button type="button" class="btn btn-ghost enter" onclick={() => openJoin(s)}>{$t('msg_srv_enter_invite')}</button>
              </span>
            {/if}
          </div>
          <button class="btn btn-ghost invite" onclick={() => privateInvite.show({ server: s.id })}>
            <Icon name="user-plus" size={18} />{$t('msg_srv_invite_row')}
          </button>
          <button class="icon-btn danger-soft" title={$t('msg_srv_remove')} aria-label={$t('msg_srv_remove')} disabled={busy} onclick={() => remove(s)}>
            <Icon name="trash-2" size={18} />
          </button>
        </li>
      {/each}
    </ul>
  {:else if serverStore.list}
    <p class="meta">{$t('msg_srv_list_empty')}</p>
  {/if}
  {#if error}<Notice tone="danger">{error}</Notice>{/if}

  <form class="add" onsubmit={(e) => { e.preventDefault(); void addByLink(); }}>
    <label class="add-label" for="srv-add-link">{$t('msg_srv_add_title')}</label>
    <div class="add-row">
      <input id="srv-add-link" type="text" inputmode="url" autocapitalize="off" autocomplete="off" spellcheck="false"
        placeholder="veydan://server/…" bind:value={link} disabled={adding} aria-invalid={!!addError} />
      <button type="submit" class="btn btn-ghost" disabled={adding || !link.trim()}>
        <Icon name={adding ? 'loader' : 'plus'} size={18} />{$t('common_add')}
      </button>
    </div>
    {#if addError}<div class="error-msg">{addError}</div>{/if}
  </form>
</div>

<!-- DS-STUB: a plain text input in a Dialog; no design-system element for a short code yet. -->
<Dialog open={joinFor !== null} title={$t('msg_srv_code_title')} width="min(400px, calc(100vw - 24px))" onclose={() => (joinFor = null)}>
  <form id="srv-join" class="add" onsubmit={(e) => { e.preventDefault(); void join(); }}>
    <label class="add-label" for="srv-join-code">{$t('msg_srv_invite_code')}</label>
    <input id="srv-join-code" type="text" autocapitalize="off" autocomplete="off" spellcheck="false" bind:value={code} disabled={joining}
      class="code" aria-invalid={!!joinError} />
    {#if joinError}<div class="error-msg">{joinError}</div>{/if}
  </form>
  {#snippet footer()}
    <button type="button" class="btn btn-ghost" onclick={() => (joinFor = null)}>{$t('common_cancel')}</button>
    <button type="submit" form="srv-join" class="btn btn-primary" disabled={joining || !code.trim()}>{$t('msg_srv_invite_join')}</button>
  {/snippet}
</Dialog>

<style>
  /* A part of the servers' card: a line above it, its rows of 48 with a line between them (as «Профиль и ключ»). */
  .private { display: flex; flex-direction: column; gap: var(--sp-2); padding-top: var(--sp-3); border-top: 1px solid var(--border); }
  .title { display: flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-sm); font-weight: var(--fw-bold); color: var(--text); }
  .title :global(svg) { flex-shrink: 0; color: var(--text-2); }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .row { display: flex; flex-wrap: wrap; align-items: center; gap: var(--sp-2) var(--sp-3); min-height: 48px; padding: var(--sp-2) 0; }
  .row + .row { border-top: 1px solid var(--border); }
  /* The words take the width of the row; «Пригласить» and the bin lie under them, to the right, never over a label. */
  .invite { flex-shrink: 0; margin-left: auto; }
  .info { flex: 1 1 100%; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-semibold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  code { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-body); overflow-wrap: anywhere; }
  .facts { display: flex; flex-wrap: wrap; gap: 4px; }
  .fact {
    display: inline-flex; align-items: center; gap: 3px; min-height: 20px; padding: 0 6px; border-radius: var(--radius-pill);
    border: 1px solid var(--border); background: var(--surface-3); color: var(--text-2); font-size: var(--fs-xs); line-height: var(--lh-snug); white-space: nowrap;
  }
  .fact.off { color: var(--text-3); border-style: dashed; background: none; }
  .meta { margin: 0; color: var(--text-3); font-size: var(--fs-xs); }
  .denied { display: flex; flex-wrap: wrap; align-items: center; gap: var(--sp-1) var(--sp-2); font-size: var(--fs-xs); color: var(--danger, var(--text-2)); }
  .enter { min-height: 32px; padding: 0 var(--sp-2); }
  .code {
    min-height: 44px; font: inherit; font-family: var(--font-mono); font-size: var(--fs-sm); color: var(--text);
    background: var(--surface-3); border: 1px solid var(--border); border-radius: var(--radius); padding: 0 var(--sp-3);
  }
  .code:focus { outline: none; border-color: var(--accent-border); }
  .add { display: flex; flex-direction: column; gap: 6px; margin-top: var(--sp-1); }
  .add-label { font-size: var(--fs-sm); font-weight: var(--fw-bold); color: var(--text-2); }
  .add-row { display: flex; gap: var(--sp-2); align-items: center; }
  .add-row input {
    flex: 1; min-width: 0; min-height: 44px; font: inherit; font-family: var(--font-mono); font-size: var(--fs-sm); color: var(--text);
    background: var(--surface-3); border: 1px solid var(--border); border-radius: var(--radius); padding: 0 var(--sp-3);
  }
  .add-row input:focus { outline: none; border-color: var(--accent-border); }
  @media (pointer: coarse) { .add-row input { font-size: var(--fs-md); } }
</style>
