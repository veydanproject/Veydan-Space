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
  }
  let { query, selected = null, onselect }: Props = $props();

  const q = $derived(query.trim());
  const haystacks = $derived(contactHaystacks(messengerStore.contacts));
  const contacts = $derived(
    messengerStore.contacts
      .filter((c) => !q || matchesQuery(haystacks.get(c.pubkey) ?? '', q))
      .sort((a, b) => contactLabel(a).localeCompare(contactLabel(b))),
  );
</script>

<div class="list-wrap">
  {#if contacts.length === 0}
    <div class="empty">
      <Icon name="book-user" size={28} />
      <p>{q ? $t('msg_chats_nothing_found') : $t('msg_contacts_empty')}</p>
      {#if !q}<span>{$t('msg_contacts_text')}</span>{/if}
    </div>
  {:else}
    <ul class="list">
      {#each contacts as c (c.pubkey)}
        <li>
          <button class="row" class:active={selected === c.pubkey} onclick={() => onselect(c)}>
            <Avatar url={c.profile?.picture ?? null} label={contactLabel(c)} seed={c.pubkey} size={42} online={presenceStore.status(c.pubkey) === 'online'} />
            <span class="body">
              <span class="name">{contactLabel(c)}</span>
              <span class="meta">{c.profile?.nip05 || `${c.npub.slice(0, 16)}…`}</span>
            </span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .list-wrap { flex: 1; min-height: 0; overflow-y: auto; padding: 0 var(--sp-2) var(--sp-2); }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  .row {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; min-height: 64px; padding: var(--sp-2) var(--sp-3);
    border: none; border-radius: var(--radius-lg); background: none; color: inherit; font: inherit; text-align: left; cursor: pointer;
    transition: background var(--dur-fast) var(--ease);
  }
  .row:hover { background: var(--surface-hover); }
  .row.active { background: var(--accent-bg); box-shadow: inset 0 0 0 1px var(--accent-border); }
  .body { display: flex; flex-direction: column; gap: 3px; min-width: 0; flex: 1; }
  .name { font-weight: var(--fw-semibold); font-size: var(--fs-body); color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .meta { font-size: var(--fs-sm); color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .empty { display: flex; flex-direction: column; align-items: center; gap: var(--sp-2); text-align: center; padding: var(--sp-8) var(--sp-4); color: var(--text-3); }
  .empty p { margin: 0; color: var(--text-body); font-weight: var(--fw-semibold); font-size: var(--fs-sm); }
  .empty span { font-size: var(--fs-xs); line-height: 1.5; }
</style>
