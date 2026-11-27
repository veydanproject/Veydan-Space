<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { messengerStore } from '../store.svelte';
  import { chatStore } from '../chats/chatStore.svelte';
  import ContactsPanel from '../contacts/ContactsPanel.svelte';
  import ContactAddForm from '../contacts/ContactAddForm.svelte';
  import MobileFrame from './MobileFrame.svelte';
  import { chatHref } from './routes';
  import { markPhone } from '../shared/phone';
  import MeButton from './MeButton.svelte';
  import MyQrButton from '../invites/MyQrButton.svelte';
  import ScanQrButton from '../invites/ScanQrButton.svelte';

  markPhone();

  let adding = $state(false);
  /** One column keeps the field always, as Chats' search. */
  let query = $state('');

  onMount(() => { messengerStore.ensureLoaded().catch(() => {}); });

  async function write(pubkey: string) {
    const chat = await chatStore.openPeer(pubkey);
    goto(chatHref(chat.id));
  }
</script>

{#snippet actions()}
  <ScanQrButton icon />
  <MyQrButton icon />
  <MeButton />
{/snippet}

<MobileFrame hubButton title={$t('msg_contacts_title')} {actions} scroll={false}>
  <div class="search">
    <Icon name="search" size={16} />
    <input type="search" bind:value={query} placeholder={$t('msg_contacts_search')} spellcheck="false" enterkeyhint="search" />
    {#if query}<button class="clear" onclick={() => (query = '')} aria-label={$t('common_back')}><Icon name="x" size={14} /></button>{/if}
  </div>
  <div class="scroller">
    <ContactsPanel compact {query} onchat={(pk) => write(pk).catch(() => {})} />
  </div>
  <button class="m-fab" onclick={() => (adding = true)} aria-label={$t('msg_contacts_new')}><Icon name="user-plus" size={22} /></button>
</MobileFrame>

<!-- On the phone a Dialog is a bottom sheet. -->
<Dialog bind:open={adding} title={$t('msg_contacts_new')}>
  <ContactAddForm stacked onadded={() => (adding = false)} />
</Dialog>

<style>
  /* Chats' search (MobileChats): the pill of 40, 8 under the head; the light frame is periwinkle's. */
  .search {
    display: flex; align-items: center; gap: var(--sp-2); margin: var(--sp-2) var(--sp-3); padding: 0 var(--sp-4); flex-shrink: 0;
    border: 1px solid var(--pane-strong-border); border-radius: var(--radius-pill); color: var(--text-3);
    /* The glass of the heads (.head.phone in ChatWindow, the owner, 2026-11-27): 60 % of --glass-bar, the pane's blur. */
    background: color-mix(in srgb, var(--glass-bar) 60%, transparent); box-shadow: var(--pane-highlight), var(--pane-shadow);
    -webkit-backdrop-filter: var(--pane-filter); backdrop-filter: var(--pane-filter);
  }
  @supports not ((backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px))) { .search { background: var(--bg-2); } }
  .search:focus-within { border-color: var(--accent-border); }
  .search input { flex: 1; min-width: 0; border: none; border-radius: 0; background: none; outline: none; box-shadow: none; font: inherit; font-size: var(--search-fs); color: var(--text); height: calc(var(--search-h) - 2px); min-height: calc(var(--search-h) - 2px); padding: 0; }
  .search input::-webkit-search-cancel-button { display: none; }
  .clear { border: none; background: none; color: var(--text-3); display: inline-flex; padding: var(--sp-2); }
  /* What MobileFrame's scrolling content does, but the «+» stays outside it; the end of the list clears the button. */
  .scroller {
    flex: 1; min-height: 0; overflow-y: auto; -webkit-overflow-scrolling: touch;
    -webkit-mask-image: linear-gradient(to bottom, transparent, black var(--sp-5)); mask-image: linear-gradient(to bottom, transparent, black var(--sp-5));
    padding: var(--sp-3) var(--sp-3) calc(52px + var(--sp-4) + var(--sp-2)); display: flex; flex-direction: column; gap: var(--sp-3);
  }
  /* The shared «+» (.m-fab), in the corner of this frame rather than over the bottom bar. */
  .m-fab { position: absolute; right: var(--sp-4); bottom: var(--sp-4); --hit-r: var(--sp-4); }
</style>
