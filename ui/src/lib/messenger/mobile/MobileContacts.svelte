<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
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
  /** The open contact's actions (ContactsPanel), kept in the page's pinned foot. */
  let foot = $state<Snippet>();

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
  <div class="scroller">
    <ContactsPanel compact bind:foot onchat={(pk) => write(pk).catch(() => {})} />
  </div>
  {#if foot}
    <div class="page-foot">{@render foot()}</div>
  {:else}
    <button class="m-fab" onclick={() => (adding = true)} aria-label={$t('msg_contacts_new')}><Icon name="user-plus" size={22} /></button>
  {/if}
</MobileFrame>

<!-- On the phone a Dialog is a bottom sheet. -->
<Dialog bind:open={adding} title={$t('msg_contacts_new')}>
  <ContactAddForm stacked onadded={() => (adding = false)} />
</Dialog>

<style>
  /* What MobileFrame's scrolling content does, but the «+» stays outside it; the end of the list clears the button. */
  .scroller {
    flex: 1; min-height: 0; overflow-y: auto; -webkit-overflow-scrolling: touch;
    -webkit-mask-image: linear-gradient(to bottom, transparent, black var(--sp-5)); mask-image: linear-gradient(to bottom, transparent, black var(--sp-5));
    padding: var(--sp-3) var(--sp-3) var(--fab-room); display: flex; flex-direction: column; gap: var(--sp-3);
  }
  /* An open contact's foot stands in place of the «+»: the list ends with the usual padding. */
  .scroller:has(+ .page-foot) { padding-bottom: var(--sp-4); }
</style>
