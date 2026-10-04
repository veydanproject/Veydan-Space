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
  import Icon from '$lib/core/Icon.svelte';
  import ConfirmHost from '../shared/ConfirmHost.svelte';
  import { markPhone } from '../shared/phone';

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
    children: Snippet;
  }
  let { title = '', onback, lead, actions, scroll = true, bare = false, children }: Props = $props();

  // What the content mounts is laid out for a phone (shared/phone.ts).
  markPhone();

  // The keyboard covers the bottom of the screen: lift everything above it.
  let kb = $state(0);
  onMount(() => onKeyboard((inset) => (kb = inset)));
</script>

<div class="m-page msg-m" class:kb-open={kb > 0} style:padding-bottom="{kb}px">
  {#if !bare}
    <header class="bar">
      {#if onback}
        <button class="ibtn" onclick={onback} aria-label={$t('msg_back')}><Icon name="chevron-left" size={24} /></button>
      {/if}
      <h1 class:inset={!onback}>{title}</h1>
      {#if lead}{@render lead()}{/if}
      <span class="spacer"></span>
      {#if actions}{@render actions()}{/if}
    </header>
  {/if}
  <div class="content" class:scroll>
    {@render children()}
  </div>
</div>
<ConfirmHost />

<style>
  .msg-m { flex: 1; min-height: 0; display: flex; flex-direction: column; background: var(--bg); }
  .bar {
    display: flex; align-items: center; gap: var(--sp-1); flex-shrink: 0; min-height: 52px;
    padding: var(--sp-1) var(--sp-2); background: var(--m-nav, var(--surface)); border-bottom: 1px solid var(--border);
  }
  h1 { margin: 0; font-size: 20px; font-weight: var(--fw-extrabold); letter-spacing: -0.02em; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  h1.inset { padding-left: var(--sp-2); }
  .spacer { flex: 1; }
  .bar :global(.ibtn), .ibtn {
    display: inline-flex; align-items: center; justify-content: center; width: 44px; height: 44px; flex-shrink: 0;
    border: none; border-radius: 12px; background: none; color: var(--text); padding: 0;
  }
  .bar :global(.ibtn:active), .ibtn:active { background: var(--surface-3); }
  .bar :global(.ibtn.accent) { color: var(--accent-text-2); }
  /* With the keyboard up the home-indicator inset is under the keyboard. */
  .kb-open { --sab: 0px; }
  .content { position: relative; flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .content.scroll { overflow-y: auto; -webkit-overflow-scrolling: touch; padding: var(--sp-3) var(--sp-3) calc(var(--sp-5) + var(--sab, 0px)); gap: var(--sp-3); }
  /* The settings' panels are the desktop's cards: on a phone they take the
     look of the shell's own cards (mobile.css .m-card) and its tighter padding. */
  .content.scroll :global(.card) {
    max-width: none; width: 100%; padding: var(--sp-4);
    background: var(--m-card, var(--surface)); border-color: var(--m-card-border, var(--border));
    border-radius: var(--m-radius, var(--radius-lg)); box-shadow: var(--m-card-shadow, none);
  }
</style>
