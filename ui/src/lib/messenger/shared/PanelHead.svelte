<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The header of a panel beside a chat (about a person, a group, what the
  chat has shared). On the desktop a panel is closed with a cross on its
  right, or left with an arrow; on a phone the panel is a screen of its own,
  and its way back is the chevron on the left, in the bar of every screen.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { onPhone } from './phone';

  interface Props {
    title: string;
    /** A step back inside the panel (an arrow on the left). */
    onback?: () => void;
    /** Closes the panel (a cross on the right). */
    onclose?: () => void;
  }
  let { title, onback, onclose }: Props = $props();

  const phone = onPhone();
  const back = $derived(onback ?? onclose);
</script>

{#if phone}
  <header class="head phone">
    {#if back}<button class="icon back" onclick={back} aria-label={$t('msg_back')}><Icon name="chevron-left" size={24} /></button>{/if}
    <span class="head-title">{title}</span>
  </header>
{:else}
  <header class="head" class:inset={!onback}>
    {#if onback}<button class="icon" onclick={onback} title={$t('msg_back')}><Icon name="arrow-left" size={16} /></button>{/if}
    <span class="head-title">{title}</span>
    {#if onclose}<button class="icon" onclick={onclose} title={$t('msg_back')}><Icon name="x" size={16} /></button>{/if}
  </header>
{/if}

<style>
  .head { display: flex; align-items: center; gap: var(--sp-2); padding: var(--sp-2) var(--sp-3); min-height: 56px; border-bottom: 1px solid var(--border); flex-shrink: 0; }
  .head.inset { padding-left: var(--sp-4); }
  .head-title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: var(--fw-bold); font-size: var(--fs-base); }
  .icon { border: none; background: none; color: var(--text-2); cursor: pointer; display: inline-flex; padding: 6px; border-radius: var(--radius-sm); }
  .icon:hover { color: var(--text); background: var(--surface-3); }
  @media (pointer: coarse) { .icon { padding: 10px; } }

  /* The phone: the bar of the messenger's screens (mobile/MobileFrame.svelte). */
  .head.phone { gap: var(--sp-1); min-height: 52px; padding: var(--sp-1) var(--sp-2) var(--sp-1) var(--sp-1); background: var(--m-nav, var(--surface)); }
  .head.phone .head-title { font-size: 20px; font-weight: var(--fw-extrabold); letter-spacing: -0.02em; }
  .head.phone .back { color: var(--text); padding: 10px; border-radius: 12px; }
  .head.phone .back:active { background: var(--surface-3); }
</style>
