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
  <header class="m-header">
    {#if back}<button class="icon-btn" onclick={back} aria-label={$t('common_back')}><Icon name="chevron-left" size={24} /></button>{/if}
    <span class="m-title">{title}</span>
  </header>
{:else}
  <header class="panel-head">
    {#if onback}<button class="icon-btn lg" onclick={onback} title={$t('common_back')}><Icon name="chevron-left" size={18} /></button>{/if}
    <span class="head-title">{title}</span>
    {#if onclose}<button class="icon-btn lg" onclick={onclose} title={$t('common_back')}><Icon name="x" size={18} /></button>{/if}
  </header>
{/if}

<style>
  /* A computer: the common .panel-head. The phone: the shell's .m-header (56, back 24), as the messenger's screens (MobileFrame). */
  .head-title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: var(--fw-bold); font-size: var(--fs-base); }
</style>
