<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A link clicked outside the app (`veydan://…`), shown over any screen: the
  same card a link in a message gets, or why it is not opened. Nothing is
  done until the card's button is pressed. The link is read by the core's
  router of links (`links` of the module, entry/common.ts) while the app is
  unlocked: one that came while it was locked is shown after.
-->
<script lang="ts">
  import { page } from '$app/state';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import LinkCard from '../content/LinkCard.svelte';
  import { incomingLinks } from './incomingStore.svelte';

  const shown = $derived(incomingLinks.shown);

  // The card's button led to a chat or a screen: the card is done. The app
  // going to its first screen at the start leaves it where it is.
  $effect(() => {
    const path = page.url.pathname;
    if (!incomingLinks.shown || path === incomingLinks.at) return;
    if (incomingLinks.pressed) incomingLinks.close();
    else incomingLinks.at = path;
  });
</script>

<Dialog open={shown !== null} title={$t('msg_inlink_title')} width="min(420px, calc(100vw - 24px))" onclose={incomingLinks.close}>
  {#if shown?.kind === 'card'}
    <!-- A new link is a new card: nothing of the one before (a question
         open, a note typed) carries over to it. -->
    {#key shown}
      <div onclickcapture={() => (incomingLinks.pressed = true)} role="presentation">
        <!-- No name of a channel asked of the link's relay before a press. -->
        <LinkCard link={{ type: 'internal', text: shown.link, link: shown.link }} preview={false} />
      </div>
    {/key}
  {:else if shown?.kind === 'note'}
    <div class="note">
      <span class="ico"><Icon name={shown.icon} size={18} /></span>
      <div class="about">
        <span class="name">{$t(shown.title)}</span>
        <span class="sub">{$t(shown.text, shown.params)}</span>
      </div>
    </div>
  {/if}
  {#snippet footer()}
    <button class="btn btn-ghost" onclick={incomingLinks.close}>{$t('common_close')}</button>
  {/snippet}
</Dialog>

<style>
  .note { display: flex; align-items: flex-start; gap: var(--sp-3); min-width: 0; }
  .ico {
    width: 38px; height: 38px; flex-shrink: 0; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center;
    background: var(--warn-bg); color: var(--warn-text);
  }
  .about { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-bold); }
  .sub { font-size: var(--fs-xs); color: var(--text-2); line-height: 1.4; overflow-wrap: anywhere; }
</style>
