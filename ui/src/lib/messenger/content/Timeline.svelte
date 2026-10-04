<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Messages of a chat, drawn: days, system lines (folded when many), runs
  of one author with the picture of whoever wrote. What a bubble and an
  album look like is for the chat to say.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Avatar from '../contacts/Avatar.svelte';
  import { nameStore } from '../groups/names.svelte';
  import { dayLabel } from '../shared/time';
  import type { MessengerMessage } from '../api';
  import { buildTimeline } from './timeline';
  import type { TimelineItem } from './types';

  type Bubble = Extract<TimelineItem, { type: 'bubble' }>;
  type Album = Extract<TimelineItem, { type: 'album' }>;

  interface Props {
    messages: MessengerMessage[];
    /** A chat of many: names and pictures of authors are shown. */
    many: boolean;
    systemText: (m: MessengerMessage) => string;
    bubble: Snippet<[Bubble]>;
    album: Snippet<[Album]>;
  }
  let { messages, many, systemText, bubble, album }: Props = $props();

  const items = $derived(buildTimeline(messages, { many }));
  /** Folded lines the user opened, by the id of the fold. */
  let opened = $state<Record<string, boolean>>({});

  function day(at: number): string {
    const l = dayLabel(at);
    return l.key ? $t(`msg_day_${l.key}` as 'msg_day_today') : l.text;
  }
</script>

{#each items as item (item.id)}
  {#if item.type === 'day'}
    <div class="day"><span>{day(item.at)}</span></div>
  {:else if item.type === 'system'}
    {#if item.messages.length === 1}
      <div class="system"><span>{systemText(item.messages[0])}</span></div>
    {:else}
      <div class="system fold">
        <button class="fold-head" aria-expanded={!!opened[item.id]} onclick={() => (opened[item.id] = !opened[item.id])}>
          {$t('msg_sys_folded', { n: String(item.messages.length) })}
          <Icon name={opened[item.id] ? 'chevron-up' : 'chevron-down'} size={12} />
        </button>
        {#if opened[item.id]}
          {#each item.messages as m (m.id)}<span>{systemText(m)}</span>{/each}
        {/if}
      </div>
    {/if}
  {:else if item.indent}
    {@const who = item.type === 'album' ? item.messages[0].sender_pubkey : item.message.sender_pubkey}
    <div class="row" class:first={item.first}>
      <div class="gutter">
        {#if item.showAvatar}<Avatar url={nameStore.picture(who)} label={nameStore.label(who)} seed={who} size={30} />{/if}
      </div>
      {#if item.type === 'album'}{@render album(item)}{:else}{@render bubble(item)}{/if}
    </div>
  {:else if item.type === 'album'}
    {@render album(item)}
  {:else}
    {@render bubble(item)}
  {/if}
{/each}

<style>
  .day, .system { display: flex; justify-content: center; margin: var(--sp-3) 0 var(--sp-1); }
  .day span { font-size: var(--fs-2xs); color: var(--text-3); background: var(--surface-2); border: 1px solid var(--border); padding: 3px 10px; border-radius: var(--radius-pill); }
  .system { margin: var(--sp-2) var(--sp-4); }
  .system span { font-size: var(--fs-xs); color: var(--text-2); text-align: center; line-height: 1.4; }
  .system.fold { flex-direction: column; align-items: center; gap: 3px; }
  .fold-head {
    display: inline-flex; align-items: center; gap: 4px; border: none; background: none; cursor: pointer;
    font: inherit; font-size: var(--fs-xs); color: var(--text-2); padding: 2px 8px; border-radius: var(--radius-pill);
  }
  .fold-head:hover { color: var(--text); background: var(--surface-3); }
  .row { display: flex; align-items: flex-end; padding-left: var(--sp-3); }
  .gutter { width: 30px; flex-shrink: 0; display: flex; padding-bottom: 1px; }
  .row > :global(.line) { flex: 1; min-width: 0; padding-left: 6px; }
  @media (pointer: coarse) {
    .row { padding-left: var(--sp-2); }
    .fold-head { padding: 8px 12px; }
  }
</style>
