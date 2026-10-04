<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  What a chat has shared, in short: the latest pictures, and a line per
  section with how many there are. A line opens the section. The same
  block in the panel about a group and about a person.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import MediaBubble from '../../media/MediaBubble.svelte';
  import { chatStore } from '../../chats/chatStore.svelte';
  import type { SharedSection } from '../../api';
  import { SECTION_ORDER, SECTIONS } from './sections';
  import { sharedStore } from './sharedStore.svelte';

  /** Pictures in the strip. */
  const STRIP = 4;

  interface Props {
    chatId: string;
    onopen: (section: SharedSection) => void;
  }
  let { chatId, onopen }: Props = $props();

  $effect(() => { const id = chatId; return untrack(() => sharedStore.watch(id)); });

  const counts = $derived(sharedStore.counts(chatId));
  const strip = $derived((sharedStore.page(chatId, 'visual')?.messages ?? []).slice(0, STRIP));

  // The strip is read once there is something for it.
  $effect(() => {
    const id = chatId;
    if (counts?.visual && !sharedStore.page(id, 'visual')) untrack(() => sharedStore.open(id, 'visual')).catch(() => {});
  });

  // What this device sends does not come back as an event: the open chat is watched too.
  let last = '';
  $effect(() => {
    if (chatStore.activeId !== chatId) return;
    const tail = chatStore.messages[chatStore.messages.length - 1];
    const now = `${chatStore.messages.length}:${tail?.id}:${tail?.status}`;
    const id = chatId;
    if (last && now !== last) untrack(() => sharedStore.changed(id));
    last = now;
  });
</script>

<section class="shared">
  <div class="label">{$t('msg_shared_title')}</div>
  <div class="box">
    {#if strip.length}
      <div class="strip">
        {#each strip as m (m.id)}
          <div class="cell"><MediaBubble message={m} variant="tile" /></div>
        {/each}
      </div>
    {/if}
    <ul class="rows">
      {#each SECTION_ORDER as s (s)}
        <li>
          <button class="row" onclick={() => onopen(s)}>
            <span class="ico"><Icon name={SECTIONS[s].icon} size={18} /></span>
            <span class="name">{$t(SECTIONS[s].label)}</span>
            <span class="count">{counts ? counts[s] : '…'}</span>
            <span class="go"><Icon name="chevron-right" size={16} /></span>
          </button>
        </li>
      {/each}
    </ul>
  </div>
</section>

<style>
  .shared { display: flex; flex-direction: column; gap: var(--sp-2); }
  .label { font-size: var(--fs-2xs); text-transform: uppercase; letter-spacing: 0.6px; color: var(--text-3); font-weight: var(--fw-bold); }
  .box { border: 1px solid var(--border); border-radius: var(--radius-lg, 14px); background: var(--surface); overflow: hidden; }
  .strip { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 4px; padding: var(--sp-3) var(--sp-3) var(--sp-2); }
  .cell { aspect-ratio: 1; border-radius: var(--radius-sm); overflow: hidden; }
  /* Small places: the picture or its colour, no file names. */
  .cell :global(.placeholder-name), .cell :global(.size), .cell :global(.center) { display: none; }
  .rows { list-style: none; margin: 0; padding: 0; }
  .rows li + li .row { border-top: 1px solid var(--border); }
  .row {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; min-height: 46px; padding: 0 var(--sp-3); border-radius: 0;
    border: none; background: none; color: var(--text); font: inherit; font-size: var(--fs-sm); text-align: left; cursor: pointer;
  }
  .row:hover { background: var(--surface-row-hover); }
  .row:focus-visible { outline: 2px solid var(--accent-border); outline-offset: -2px; }
  .ico { display: inline-flex; color: var(--accent-text-2); }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .count { color: var(--text-3); font-variant-numeric: tabular-nums; }
  .go { display: inline-flex; color: var(--text-3); }
  @media (pointer: coarse) { .row { min-height: 50px; } }
</style>
