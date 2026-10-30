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
      <button class="dial" disabled={!!block} onclick={() => call(c, 'audio')} title={$t('msg_calls_call')} aria-label={$t('msg_calls_call')}><CallIcon name="phone" size={18} /></button>
    {:else}
      <button class="dial" disabled={!!block} onclick={() => call(c, 'video')} title={$t('msg_call_video_start')} aria-label={$t('msg_call_video_start')}><CallIcon name="video" size={18} /></button>
      <button class="dial" disabled={!!block} onclick={() => call(c, 'audio')} title={$t('msg_call_start')} aria-label={$t('msg_call_start')}><CallIcon name="phone" size={18} /></button>
    {/if}
  </li>
{/snippet}

<Dialog bind:open title={$t('msg_calls_new')} width="min(440px, calc(100vw - 24px))">
  <div class="body">
    <div class="search">
      <Icon name="search" size={16} />
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
  /* The field of whom: 44 high with the search glyph inside (48 under a finger), the ring around the whole field. */
  .search {
    display: flex; align-items: center; gap: var(--sp-2); min-height: var(--control-h-lg); padding: 0 var(--sp-3);
    background: var(--surface-3); border: 1px solid var(--border); border-radius: var(--radius); color: var(--text-3);
  }
  .search:focus-within { border-color: var(--accent-border); box-shadow: 0 0 0 3px var(--accent-bg); }
  .search :global(svg) { flex-shrink: 0; }
  .search input { flex: 1; min-width: 0; align-self: stretch; padding: 0; border: none; background: none; font: inherit; font-size: var(--fs-base); color: var(--text); }
  .search input:focus { outline: none; box-shadow: none; }
  /* «Контакты», «Группы» in sentence case over their cards. */
  .label { margin-top: var(--sp-1); font-size: var(--fs-base); font-weight: var(--fw-bold); color: var(--text); }
  /* A card of rows; a line between rows after the avatar. */
  .list {
    list-style: none; margin: 0; padding: var(--sp-1); display: flex; flex-direction: column;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-lg);
  }
  .row { position: relative; display: flex; align-items: center; gap: var(--sp-3); min-height: 48px; padding: var(--sp-1) var(--sp-1) var(--sp-1) var(--sp-3); border-radius: var(--radius-md); }
  .row + .row::before { content: ''; position: absolute; top: 0; left: calc(var(--sp-12) + var(--sp-2)); right: var(--sp-3); height: 1px; background: var(--border-2); }
  @media (hover: hover) { .row:hover { background: var(--surface-row-hover); } .row:hover::before, .row:hover + .row::before { opacity: 0; } }
  .name { flex: 1; min-width: 0; font-size: var(--fs-base); font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dial {
    width: 36px; height: 36px; flex-shrink: 0; padding: 0; border: none; border-radius: var(--radius); background: none; color: var(--success-text);
    display: inline-flex; align-items: center; justify-content: center; cursor: pointer;
  }
  @media (hover: hover) { .dial:hover:not(:disabled) { background: var(--success-bg); } }
  .dial:focus-visible { outline: 2px solid var(--accent-border); outline-offset: 2px; }
  .dial:disabled { opacity: 0.45; cursor: default; }
  .hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.5; }
  @media (pointer: coarse) { .search { min-height: 48px; } .dial { width: 44px; height: 44px; } }
</style>
