<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A row of a section a module gives (seams/sections: a channel), the whole
  screen of a phone: the module's page inside the messenger's frame, which
  brings the confirmation and the report its page asks for. A row of a
  module that is off (or not in the product) has nothing to show.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { t } from '$lib/core/i18n';
  import { leave } from '$lib/core/ui/back';
  import MobileFrame from './MobileFrame.svelte';
  import { BASE, openHref, sectionHref, sectionItemHref } from './routes';
  import { onChatOpened } from '../content/actions';
  import { chatSections } from '../seams/sections.svelte';
  import { markPhone } from '../shared/phone';

  markPhone();

  const id = $derived(page.url.searchParams.get('id') ?? '');
  const host = $derived(chatSections.ofChat(id));

  // A chat opened from the row's page («Написать автору») is a page of its own on a phone.
  onMount(() => onChatOpened((chatId) => goto(openHref(chatId, !!chatSections.ofChat(chatId)))));

  function back() {
    void leave(host ? sectionHref(host.id) : BASE);
  }
</script>

{#if host && id}
  {@const View = host.view}
  <MobileFrame bare scroll={false}>
    <View chatId={id} onback={back} onopen={(other) => goto(sectionItemHref(other), { replaceState: true })} />
  </MobileFrame>
{:else}
  <MobileFrame title={$t('msg_title')} onback={() => goto(BASE, { replaceState: true })}>
    <div class="note">{$t('msg_mobile_chat_missing')}</div>
  </MobileFrame>
{/if}

<style>
  .note { padding: var(--sp-6) var(--sp-4); text-align: center; color: var(--text-2); font-size: var(--fs-sm); }
</style>
