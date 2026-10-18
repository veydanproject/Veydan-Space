<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «Добавить сервер?» (askServers): what will be added, server by server —
  its name, whether it is Veydan's, its relay, media, calls and push — with
  «Добавить» and «Отмена». Mount once per page, beside ConfirmHost; every
  path that adds a server asks here first, and nothing is added on «Отмена».
-->
<script lang="ts">
  import { get } from 'svelte/store';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { serverAcceptState as s, serverFacts, serverName } from './serverAccept.svelte';

  const tr = (key: string, params?: Record<string, string>) => get(t)(key as 'msg_you', params);
  const open = $derived(s.current !== null);
</script>

<Dialog {open} title={$t('msg_srv_accept_title')} width="min(440px, calc(100vw - 24px))" closable={false} onclose={() => s.answer(false)}>
  {#if s.current}
    <div class="body">
      <p class="lead">{$t('msg_srv_accept_lead')}</p>
      <ul class="list">
        {#each s.current.servers as server (server.id)}
          <li class="server">
            <div class="head">
              <Icon name={server.is_veydan ? 'shield-check' : 'server-2'} size={18} />
              <span class="name">{serverName(server, tr)}</span>
              <span class="whose" class:veydan={server.is_veydan}>{$t(server.is_veydan ? 'msg_srv_is_veydan' : 'msg_srv_not_veydan')}</span>
            </div>
            <ul class="facts">
              {#each serverFacts(server, tr) as fact (fact)}<li>{fact}</li>{/each}
            </ul>
          </li>
        {/each}
      </ul>
      <p class="note">{$t('msg_srv_accept_note')}</p>
    </div>
  {/if}
  {#snippet footer()}
    <button type="button" class="btn btn-ghost" onclick={() => s.answer(false)}>{$t('common_cancel')}</button>
    <button type="button" class="btn btn-primary" onclick={() => s.answer(true)}>{$t('msg_srv_accept_yes')}</button>
  {/snippet}
</Dialog>

<style>
  .body { display: flex; flex-direction: column; gap: var(--sp-3); }
  .lead { margin: 0; font-size: var(--fs-sm); color: var(--text-body); }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-2); }
  .server { display: flex; flex-direction: column; gap: 6px; padding: var(--sp-3); border: 1px solid var(--border); border-radius: var(--radius-sm); background: var(--surface-2); }
  .head { display: flex; align-items: center; gap: var(--sp-2); min-width: 0; color: var(--text-2); }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-bold); color: var(--text); flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .whose { font-size: var(--fs-2xs); color: var(--warn-text); white-space: nowrap; }
  .whose.veydan { color: var(--success-text); }
  .facts { margin: 0; padding-inline-start: 28px; display: flex; flex-direction: column; gap: 2px; font-size: var(--fs-xs); color: var(--text-2); line-height: 1.4; overflow-wrap: anywhere; }
  .note { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.45; }
</style>
