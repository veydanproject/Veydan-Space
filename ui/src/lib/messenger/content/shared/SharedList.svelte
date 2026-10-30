<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  One section of what a chat has shared, newest first, by month:
  pictures as a grid, files as cards, links as the cards a chat shows
  under a message, voice and round videos as their players.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import PanelHead from '../../shared/PanelHead.svelte';
  import MediaBubble from '../../media/MediaBubble.svelte';
  import LinkCard from '../LinkCard.svelte';
  import { nameStore } from '../../groups/names.svelte';
  import type { MessengerMessage, SharedSection } from '../../api';
  import { linkItems, SECTION_ORDER, SECTIONS } from './sections';
  import { sharedStore } from './sharedStore.svelte';
  import { monthTitle, stamp } from '../../shared/time';

  interface Props {
    chatId: string;
    /** The section shown first; the tabs change it. */
    section: SharedSection;
    onback: () => void;
  }
  let { chatId, section: first, onback }: Props = $props();
  let section = $state<SharedSection>(untrack(() => first));

  $effect(() => { const id = chatId; return untrack(() => sharedStore.watch(id)); });
  $effect(() => { const id = chatId, s = section; untrack(() => sharedStore.open(id, s)).catch(() => {}); });

  const page = $derived(sharedStore.page(chatId, section));
  const messages = $derived(page?.messages ?? []);

  interface Month<T> { key: string; title: string; items: T[] }

  function byMonth<T>(items: T[], at: (item: T) => number): Month<T>[] {
    const out: Month<T>[] = [];
    for (const item of items) {
      const d = new Date(at(item) * 1000);
      const key = `${d.getFullYear()}-${d.getMonth()}`;
      if (out[out.length - 1]?.key !== key) out.push({ key, title: monthTitle(at(item)), items: [] });
      out[out.length - 1].items.push(item);
    }
    return out;
  }

  const months = $derived(byMonth(messages, (m) => m.created_at));
  const linkMonths = $derived(byMonth(linkItems(messages), (i) => i.message.created_at));

  function meta(m: MessengerMessage): string {
    const who = m.direction === 'out' ? $t('msg_you') : nameStore.label(m.sender_pubkey);
    return `${who} · ${stamp(m.created_at)}`;
  }

  /** Older ones are read when the end of the list comes into sight. */
  function nearEnd(node: HTMLElement) {
    const io = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) sharedStore.more(chatId, section).catch(() => {});
    }, { rootMargin: '200px' });
    io.observe(node);
    return { destroy: () => io.disconnect() };
  }
</script>

<div class="panel">
  <PanelHead title={$t('msg_shared_title')} {onback} />

  <div class="seg tabs" role="tablist">
    {#each SECTION_ORDER as s (s)}
      <button class="seg-btn" class:active={s === section} role="tab" aria-selected={s === section} onclick={() => (section = s)}>
        {$t(SECTIONS[s].label)}
      </button>
    {/each}
  </div>

  <div class="scroll">
    {#if !messages.length}
      {#if page?.loading || !page}
        <div class="empty"><Icon name="loader" size={16} /></div>
      {:else}
        <div class="empty"><Icon name={SECTIONS[section].icon} size={22} /><span>{$t(SECTIONS[section].empty)}</span></div>
      {/if}
    {:else if section === 'links'}
      {#each linkMonths as mo (mo.key)}
        <div class="month">{mo.title}</div>
        <ul class="list">
          {#each mo.items as it (`${it.message.id}|${it.link.text}`)}
            <li class="item"><span class="meta">{meta(it.message)}</span><LinkCard link={it.link} /></li>
          {/each}
        </ul>
      {/each}
    {:else}
      {#each months as mo (mo.key)}
        <div class="month">{mo.title}</div>
        {#if section === 'visual'}
          <div class="grid">
            {#each mo.items as m (m.id)}<div class="cell"><MediaBubble message={m} variant="tile" /></div>{/each}
          </div>
        {:else}
          <ul class="list">
            {#each mo.items as m (m.id)}
              <li class="item">
                <span class="meta">{meta(m)}</span>
                {#if section === 'files'}<MediaBubble message={m} variant="card" wide />{:else}<MediaBubble message={m} />{/if}
              </li>
            {/each}
          </ul>
        {/if}
      {/each}
    {/if}
    {#if page?.more}
      <div class="end" use:nearEnd>
        <button class="btn btn-ghost btn-sm" disabled={page.loading} onclick={() => sharedStore.more(chatId, section)}>
          {#if page.loading}<Icon name="loader" size={13} />{/if}{$t('msg_shared_more')}
        </button>
      </div>
    {/if}
  </div>
</div>

<style>
  .panel { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--surface); }
  /* The sections: one row of segments under the head (design book, «Медиа и файлы»). */
  .tabs { display: flex; flex-shrink: 0; gap: 2px; margin: var(--sp-3) var(--sp-4) 0; overflow-x: auto; scrollbar-width: none; }
  .tabs .seg-btn { flex: 1 0 auto; justify-content: center; height: 34px; padding: 0 var(--sp-1); font-size: var(--fs-sm); white-space: nowrap; }
  .tabs .seg-btn:focus-visible { outline: 2px solid var(--accent-border); outline-offset: -2px; }
  .scroll { flex: 1; min-height: 0; overflow-y: auto; padding: var(--sp-3) var(--sp-4); display: flex; flex-direction: column; gap: var(--sp-3); }
  .month { font-size: var(--fs-sm); color: var(--text-2); font-weight: var(--fw-bold); padding-top: var(--sp-2); }
  .month:first-child { padding-top: 0; }
  /* «октябрь 2026 г.» of the calendar starts with a capital, as a caption (design book). */
  .month::first-letter { text-transform: uppercase; }
  .grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: var(--sp-1); }
  .cell { aspect-ratio: 1; overflow: hidden; border-radius: var(--radius); }
  .cell :global(.placeholder-name) { display: none; }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-3); }
  .item { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  /* Its own line height, never squeezed by the column: descenders and commas stay whole */
  .meta { flex: none; line-height: 1.5; font-size: var(--fs-xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .item { flex: none; }
  .empty { display: flex; flex-direction: column; align-items: center; gap: var(--sp-2); padding: var(--sp-6, 32px) var(--sp-4); color: var(--text-3); font-size: var(--fs-xs); text-align: center; }
  .empty :global(svg) { opacity: 0.7; }
  .end { display: flex; justify-content: center; padding: var(--sp-2) 0; }
  @media (pointer: coarse) { .tabs .seg-btn { height: 40px; } }
</style>
