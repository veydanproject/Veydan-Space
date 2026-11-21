<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  One phone screen of the messenger: a bar with an optional back button,
  a title and actions, then the content. The root carries the host's
  `m-page` class only to take the height the mobile shell gives a page.
-->
<script lang="ts">
  import { onMount, type Snippet } from 'svelte';
  import '../shared/emoji/font.css';
  import { onKeyboard } from '../shared/keyboard';
  import { t } from '$lib/core/i18n';
  import HubButton from '$lib/core/ui/HubButton.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import ReportHost from '../shared/ReportHost.svelte';
  import ServerAcceptDialog from '../servers/ServerAcceptDialog.svelte';
  import ServerInfoDialog from '../servers/ServerInfoDialog.svelte';
  import PrivateInviteDialog from '../servers/PrivateInviteDialog.svelte';
  import { markPhone } from '../shared/phone';
  import { PHONE_OS } from '../phone';

  interface Props {
    title?: string;
    onback?: () => void;
    /** Left of the title (badge, status dot). */
    lead?: Snippet;
    actions?: Snippet;
    /** `false` lets the content manage its own scrolling (chat). */
    scroll?: boolean;
    /** Hide the bar (the chat brings its own header). */
    bare?: boolean;
    /** The product's apps button left of the title (a tab of the bar, which has no logo). */
    hubButton?: boolean;
    children: Snippet;
  }
  let { title = '', onback, lead, actions, scroll = true, bare = false, hubButton = false, children }: Props = $props();

  // What the content mounts is laid out for a phone (shared/phone.ts).
  markPhone();

  // The keyboard covers the bottom of the screen: lift everything above it.
  let kb = $state(0);
  onMount(() => onKeyboard((inset) => (kb = inset)));
</script>

<div class="m-page msg-m" class:bare class:kb-open={kb > 0} class:ios={PHONE_OS === 'ios'} style:padding-bottom="{kb}px">
  {#if !bare}
    <header class="m-header">
      {#if onback}
        <button class="icon-btn" onclick={onback} aria-label={$t('common_back')}><Icon name="chevron-left" size={24} /></button>
      {/if}
      {#if hubButton && !onback}<HubButton />{/if}
      <h1 class="m-title">{title}</h1>
      {#if lead}{@render lead()}{/if}
      <span class="spacer"></span>
      {#if actions}{@render actions()}{/if}
    </header>
  {/if}
  <div class="content" class:scroll>
    {@render children()}
  </div>
</div>
<ReportHost />
<ServerAcceptDialog />
<ServerInfoDialog />
<PrivateInviteDialog />

<style>
  .msg-m { flex: 1; min-height: 0; display: flex; flex-direction: column; background: transparent; }
  /* A chat, a group, a channel bring their own head, glass over the conversation: no fade in. While a page's
     opacity is under 1 the WebView draws no blur behind that glass, the first 0.2 s the posts showed through it
     as if over the head, and then the head as if over them (ROO-558). */
  .msg-m.bare { animation: none; }
  /* The head is the shell's .m-header (56, back 24); a title next to a lead or the actions keeps its own width. */
  .m-header > .m-title { flex: 0 1 auto; }
  /* With the keyboard up the home-indicator inset is under the keyboard. */
  .kb-open { --sab: 0px; }
  /* iOS gives the whole height of the keyboard before the keyboard slides in: on the iPhone the page rises
     with it (about its time and curve) instead of jumping above it at once. */
  .msg-m.ios { transition: padding-bottom 260ms cubic-bezier(0.38, 0.7, 0.125, 1); }
  .content { position: relative; flex: 1; min-height: 0; display: flex; flex-direction: column; }
  /* A tablet, a phone on its side: a column of 560 at most in the middle, as the shell's .m-body (mobile.css); the content still scrolls the whole width. */
  .content.scroll { -webkit-mask-image: linear-gradient(to bottom, transparent, black var(--sp-5)); mask-image: linear-gradient(to bottom, transparent, black var(--sp-5)); overflow-y: auto; -webkit-overflow-scrolling: touch; padding: var(--sp-3) max(var(--sp-3), calc((100% - 560px) / 2 + var(--sp-3))) calc(var(--sp-5) + var(--sab, 0px)); gap: var(--sp-3); }
  /* The settings' panels are the desktop's cards: on a phone they take the
     look of the shell's own cards (mobile.css .m-card) and the padding of the
     settings' cards on a computer (SettingsView): the same card, one padding. */
  .content.scroll :global(.card) {
    max-width: none; width: 100%; padding: var(--sp-4) var(--sp-5);
    background: var(--pane-strong); border-color: var(--m-card-border, var(--border));
    border-radius: var(--radius-card); box-shadow: var(--m-card-shadow, none);
  }
</style>
