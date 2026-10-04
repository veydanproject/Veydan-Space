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
  import { BASE, chatHref } from './routes';
  import { markPhone } from '../shared/phone';

  markPhone();

  let adding = $state(false);

  onMount(() => { messengerStore.ensureLoaded().catch(() => {}); });

  async function write(pubkey: string) {
    const chat = await chatStore.openPeer(pubkey);
    goto(chatHref(chat.id));
  }
</script>

{#snippet actions()}
  <button class="ibtn" onclick={() => (adding = true)} aria-label={$t('msg_contacts_new')}><Icon name="user-plus" size={21} /></button>
{/snippet}

<MobileFrame title={$t('msg_contacts_title')} onback={() => goto(BASE)} {actions}>
  <ContactsPanel compact onchat={(pk) => write(pk).catch(() => {})} />
</MobileFrame>

<!-- On the phone a Dialog is a bottom sheet. -->
<Dialog bind:open={adding} title={$t('msg_contacts_new')}>
  <ContactAddForm stacked onadded={() => (adding = false)} />
</Dialog>
