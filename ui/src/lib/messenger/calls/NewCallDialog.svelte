<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «Новый звонок»: whom to call, found by name — the contacts I am in a
  mutual chat with (a call or a video call) and my groups (a group call).
  Only those can be called; anyone else is written to first.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import { chatStore } from '../chats/chatStore.svelte';
  import { messengerStore } from '../store.svelte';
  import { chatMatches, contactHaystacks } from '../shared/search';
  import { onPhone } from '../shared/phone';
  import type { CallMedia, MessengerChat } from '../api';
  import CallIcon from './CallIcon.svelte';
  import { dialable, dialBlock, dialer, targetOf } from './dial.svelte';

  let { open = $bindable() }: { open: boolean } = $props();

  const phone = onPhone();
  let query = $state('');
  $effect(() => { if (!open) query = ''; });

  const q = $derived(query.trim());
  const haystacks = $derived(contactHaystacks(messengerStore.contacts));
  const chats = $derived(chatStore.chats.filter((c) => c.mode !== 'request_received' && dialable(c) && chatMatches(c, q, haystacks)));
  const people = $derived(chats.filter((c) => c.kind !== 'group'));
  const groups = $derived(chats.filter((c) => c.kind === 'group'));
  const block = $derived(open ? dialBlock() : '');

  function call(c: MessengerChat, media: CallMedia) {
    open = false;
    dialer.call(targetOf(c), media, phone);
  }
</script>

{#snippet row(c: MessengerChat)}
  <li class="row">
    <Avatar url={c.picture} label={c.title} seed={c.peer_pubkey ?? c.id} size={36} />
    <span class="name">{c.title}</span>
    {#if c.kind === 'group'}
      <button class="dial" disabled={!!block} onclick={() => call(c, 'audio')} title={$t('msg_calls_call')} aria-label={$t('msg_calls_call')}><CallIcon name="phone" size={17} /></button>
    {:else}
      <button class="dial" disabled={!!block} onclick={() => call(c, 'video')} title={$t('msg_call_video_start')} aria-label={$t('msg_call_video_start')}><CallIcon name="video" size={17} /></button>
      <button class="dial" disabled={!!block} onclick={() => call(c, 'audio')} title={$t('msg_call_start')} aria-label={$t('msg_call_start')}><CallIcon name="phone" size={17} /></button>
    {/if}
  </li>
{/snippet}

<Dialog bind:open title={$t('msg_calls_new')} width="min(440px, calc(100vw - 24px))">
  <div class="body">
    <div class="search">
      <Icon name="search" size={14} />
      <input type="text" bind:value={query} placeholder={$t('msg_calls_new_search')} aria-label={$t('msg_calls_new_search')} spellcheck="false" />
    </div>
    {#if block}<p class="hint">{block}</p>{/if}
    {#if people.length}
      <div class="label">{$t('msg_contacts_title')}</div>
      <ul class="list">{#each people as c (c.id)}{@render row(c)}{/each}</ul>
    {/if}
    {#if groups.length}
      <div class="label">{$t('msg_nav_groups')}</div>
      <ul class="list">{#each groups as c (c.id)}{@render row(c)}{/each}</ul>
    {/if}
    {#if !people.length && !groups.length}
      <p class="hint">{q ? $t('msg_chats_nothing_found') : $t('msg_calls_new_empty')}</p>
    {/if}
  </div>
</Dialog>

<style>
  .body { display: flex; flex-direction: column; gap: var(--sp-3); }
  .search {
    display: flex; align-items: center; gap: 8px; min-height: 40px; padding: 0 var(--sp-4);
    background: var(--surface-3); border: 1px solid var(--border); border-radius: var(--radius-pill); color: var(--text-3);
  }
  .search:focus-within { border-color: var(--accent-border); }
  .search input { flex: 1; min-width: 0; border: none; background: none; outline: none; box-shadow: none; font: inherit; font-size: var(--fs-base); color: var(--text); padding: 7px 0; }
  .label { font-size: var(--fs-2xs); text-transform: uppercase; letter-spacing: 0.6px; color: var(--text-3); font-weight: var(--fw-bold); }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .row { display: flex; align-items: center; gap: var(--sp-3); min-height: 48px; padding: 4px 4px 4px 8px; border-radius: var(--radius-sm); }
  .row:hover { background: var(--surface-row-hover); }
  .name { flex: 1; min-width: 0; font-size: var(--fs-sm); font-weight: var(--fw-semibold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dial {
    width: 40px; height: 40px; flex-shrink: 0; border: none; border-radius: 50%; background: none; color: var(--success-text);
    display: inline-flex; align-items: center; justify-content: center; cursor: pointer;
  }
  @media (hover: hover) { .dial:hover:not(:disabled) { background: var(--success-bg); } }
  .dial:disabled { opacity: 0.45; cursor: default; }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.5; }
</style>
