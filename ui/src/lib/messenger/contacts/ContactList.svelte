<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The address book in the list's column of a computer: one row per contact,
  as the chats are listed, found by name, NIP-05 or key. A press opens the
  contact in the pane (ContactDetails).
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { contactLabel, type MessengerContact } from '../api';
  import { messengerStore } from '../store.svelte';
  import { presenceStore } from '../presence/presenceStore.svelte';
  import { contactHaystacks, matchesQuery } from '../shared/search';
  import Avatar from './Avatar.svelte';

  interface Props {
    query: string;
    /** The contact open in the pane (its key). */
    selected?: string | null;
    onselect: (c: MessengerContact) => void;
    /** The column is its covers (72 px, a computer): an avatar per contact, the name its tooltip. */
    covers?: boolean;
  }
  let { query, selected = null, onselect, covers = false }: Props = $props();

  const q = $derived(query.trim());
  const haystacks = $derived(contactHaystacks(messengerStore.contacts));
  const contacts = $derived(
    messengerStore.contacts
      .filter((c) => !q || matchesQuery(haystacks.get(c.pubkey) ?? '', q))
      .sort((a, b) => contactLabel(a).localeCompare(contactLabel(b))),
  );
</script>

{#if covers}
  <div class="covers">
    {#if contacts.length === 0}
      <span class="list-cover-empty" role="status" aria-label={q ? $t('msg_chats_nothing_found') : $t('msg_contacts_empty')} title={q ? $t('msg_chats_nothing_found') : $t('msg_contacts_empty')}><Icon name="book-user" size={22} /></span>
    {:else}
      <ul class="covers-list">
        {#each contacts as c (c.pubkey)}
          <li>
            <button class="list-cover" class:active={selected === c.pubkey} title={contactLabel(c)} aria-label={contactLabel(c)} onclick={() => onselect(c)}>
              <Avatar url={c.profile?.picture ?? null} label={contactLabel(c)} seed={c.pubkey} size={42} online={presenceStore.status(c.pubkey) === 'online'} />
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
{:else}
<div class="list-wrap">
  {#if contacts.length === 0}
    <div class="empty">
      <Icon name="book-user" size={28} />
      <p>{q ? $t('msg_chats_nothing_found') : $t('msg_contacts_empty')}</p>
      {#if !q}<span>{$t('msg_contacts_text')}</span>{/if}
    </div>
  {:else}
    <ul class="list-rows">
      {#each contacts as c (c.pubkey)}
        <li>
          <button class="list-row" class:active={selected === c.pubkey} onclick={() => onselect(c)}>
            <Avatar url={c.profile?.picture ?? null} label={contactLabel(c)} seed={c.pubkey} size={48} online={presenceStore.status(c.pubkey) === 'online'} />
            <span class="list-row-body">
              <span class="list-row-top"><span class="list-row-name">{contactLabel(c)}</span></span>
              <span class="list-row-bottom"><span class="list-row-line">{c.profile?.nip05 || `${c.npub.slice(0, 16)}…`}</span></span>
            </span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>
{/if}

<style>
  .list-wrap { flex: 1; min-height: 0; overflow-y: auto; padding: 0 var(--sp-2) var(--sp-2); }
  /* The rows are the .list-row of base.css, as the chats: 64, an avatar of 48, a hairline between two rows. */
  .empty { display: flex; flex-direction: column; align-items: center; gap: var(--sp-2); text-align: center; padding: var(--sp-8) var(--sp-4); color: var(--text-3); }
  .empty p { margin: 0; color: var(--text-body); font-weight: var(--fw-semibold); font-size: var(--fs-sm); }
  .empty span { font-size: var(--fs-xs); line-height: 1.5; }
  /* The column of covers: the open one lit. */
  .covers { flex: 1; min-height: 0; overflow-y: auto; scrollbar-width: none; display: flex; flex-direction: column; align-items: center; padding: var(--sp-1) 0 var(--sp-3); }
  .covers-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; align-items: center; gap: var(--sp-1); }
</style>
