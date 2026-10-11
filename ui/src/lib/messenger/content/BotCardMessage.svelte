<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A card of the bot of Veydan Cloud in its chat (messenger-wire.md §3). The
  runtime says which message is one (`bot_card`, only what the pinned key
  sealed) and hands its typed values; the words and the button are of the
  module that took the cards. A type nobody knows, and every card in a
  product without that module, is «Сообщение Veydan Cloud» without text.
  Nothing here is a text of the card: no link is ever made of it.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import type { BotCard } from '../generated/bot';
  import { botCards } from '../seams/cards.svelte';
  import { drawnByHost, NEUTRAL } from './botCard';

  interface Props {
    card: BotCard;
    /** The id of the message. */
    id: string;
    at: number;
  }
  let { card, id, at }: Props = $props();

  const host = $derived(botCards.host);
</script>

{#if drawnByHost(card, host)}
  {@const Card = host.card}
  <Card {card} {id} {at} />
{:else}
  <div class="neutral">
    <span class="ico"><Icon name="info" size={16} /></span>
    <span class="name">{$t(NEUTRAL)}</span>
  </div>
{/if}

<style>
  .neutral { display: flex; align-items: center; gap: var(--sp-2); min-width: 0; }
  .ico {
    width: 30px; height: 30px; flex-shrink: 0; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center;
    background: var(--surface-3); color: var(--text-2);
  }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-semibold); }
</style>
