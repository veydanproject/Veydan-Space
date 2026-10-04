<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import Icon from '$lib/core/Icon.svelte';
  import { api, formatError, type NoteSearchResult, type NoteTag } from '$lib/notes/mobile/api';
  import { locale, t } from '$lib/core/mobile/i18n';
  import NotesBurger from '$lib/notes/mobile/NotesBurger.svelte';
  import { fmtListDate, noteColor } from '$lib/notes/mobile/notes-editor';

  type Scope = 'all' | 'notes' | 'tags';

  let query = $state('');
  let scope = $state<Scope>('all');
  let result = $state<NoteSearchResult | null>(null);
  let tags = $state<NoteTag[]>([]);
  let showAll = $state(false);
  let error = $state('');
  let timer: ReturnType<typeof setTimeout>;

  const q = $derived(query.trim());
  const tagHits = $derived(q ? tags.filter((x) => x.name.toLowerCase().includes(q.toLowerCase())) : []);
  const notes = $derived(result?.notes ?? []);
  const shownNotes = $derived(showAll || scope === 'notes' ? notes : notes.slice(0, 3));
  const textHits = $derived(result?.text_matches ?? []);
  const empty = $derived(!!q && !!result && notes.length === 0 && textHits.length === 0 && tagHits.length === 0);

  function onInput() {
    clearTimeout(timer);
    showAll = false;
    if (!q) {
      result = null;
      return;
    }
    timer = setTimeout(run, 200);
  }

  async function run() {
    try {
      result = await api.notes.search(q);
    } catch (e) {
      error = formatError(e);
    }
  }

  /** Split text around case-insensitive hits so the match can be highlighted. */
  function parts(text: string): { s: string; hit: boolean }[] {
    if (!q) return [{ s: text, hit: false }];
    const out: { s: string; hit: boolean }[] = [];
    const lower = text.toLowerCase();
    const needle = q.toLowerCase();
    let i = 0;
    while (i < text.length) {
      const j = lower.indexOf(needle, i);
      if (j < 0) {
        out.push({ s: text.slice(i), hit: false });
        break;
      }
      if (j > i) out.push({ s: text.slice(i, j), hit: false });
      out.push({ s: text.slice(j, j + needle.length), hit: true });
      i = j + needle.length;
    }
    return out;
  }

  function cancel() {
    if (history.length > 1) history.back();
    else void goto('/');
  }

  onMount(() => {
    api.notes.tags().then((x) => (tags = x)).catch(() => {});
  });
</script>

{#snippet hl(text: string)}
  {#each parts(text) as p, i (i)}{#if p.hit}<mark class="m-highlight">{p.s}</mark>{:else}{p.s}{/if}{/each}
{/snippet}

<div class="m-page">
  <div class="m-header">
    <NotesBurger />
    <h1 class="m-title">{$t('nav_search')}</h1>
  </div>
  <div class="m-search">
    <div class="field">
      <Icon name="search" size={16} />
      <input type="search" bind:value={query} oninput={onInput} placeholder={$t('search_placeholder')} />
      {#if query}
        <button type="button" class="clear" onclick={() => { query = ''; onInput(); }} aria-label={$t('common_clear')}><Icon name="x" size={16} /></button>
      {/if}
    </div>
    <button type="button" class="cancel" onclick={cancel}>{$t('common_cancel')}</button>
  </div>

  <div class="m-seg">
    <button type="button" class="m-seg-btn" class:active={scope === 'all'} onclick={() => (scope = 'all')}>{$t('search_all')}</button>
    <button type="button" class="m-seg-btn" class:active={scope === 'notes'} onclick={() => (scope = 'notes')}>{$t('search_notes')}</button>
    <button type="button" class="m-seg-btn" class:active={scope === 'tags'} onclick={() => (scope = 'tags')}>{$t('search_tags')}</button>
  </div>

  <div class="m-body">
  {#if error}<div class="m-error">{error}</div>{/if}

  {#if !q}
    <div class="m-empty"><Icon name="search" size={40} /><p>{$t('search_hint')}</p></div>
  {:else if empty}
    <div class="m-empty"><Icon name="search" size={40} /><p>{$t('common_nothing_found')}</p></div>
  {:else}
    {#if scope !== 'tags' && notes.length}
      <div class="sec">
        <h3>{$t('search_notes')} ({notes.length})</h3>
        {#if !showAll && scope === 'all' && notes.length > 3}
          <button type="button" class="more" onclick={() => (showAll = true)}>{$t('search_show_all')}</button>
        {/if}
      </div>
      <div class="m-cards">
        {#each shownNotes as n (n.id)}
          <a class="m-card m-note compact" href="/notes/{n.id}" style:--c={noteColor(n.chips)}>
            <span class="m-doc"><Icon name="file-text" size={16} /></span>
            <span class="body">
              <span class="top">
                <span class="title">{@render hl(n.title || $t('notes_untitled'))}</span>
                <span class="date">{fmtListDate(n.updated_at, $locale)}</span>
              </span>
              {#if n.preview}<span class="preview">{n.preview}</span>{/if}
            </span>
          </a>
        {/each}
      </div>
    {/if}

    {#if scope !== 'notes' && tagHits.length}
      <div class="sec"><h3>{$t('search_tags')} ({tagHits.length})</h3></div>
      <div class="m-list">
        {#each tagHits as tag (tag.id)}
          <a class="m-row" href="/notes?kind=tag&id={encodeURIComponent(tag.name)}">
            <span class="m-dot" style:background={tag.color}></span>
            <span class="m-row-label">{@render hl(tag.name)}</span>
            <span class="count">{tag.count}</span>
          </a>
        {/each}
      </div>
    {/if}

    {#if scope !== 'tags' && textHits.length}
      <div class="sec"><h3>{$t('search_text_matches')}</h3></div>
      <div class="m-cards">
        {#each textHits as m (m.id)}
          <a class="m-card m-note compact" href="/notes/{m.id}" style:--c={noteColor(m.chips)}>
            <span class="m-doc"><Icon name="file-text" size={16} /></span>
            <span class="body">
              <span class="top">
                <span class="title">{m.title || $t('notes_untitled')}</span>
                <span class="date">{fmtListDate(m.updated_at, $locale)}</span>
              </span>
              <span class="preview">{@render hl(m.snippet)}</span>
            </span>
          </a>
        {/each}
      </div>
    {/if}
  {/if}
  </div>
</div>

<style>
  .sec {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--sp-4) var(--sp-1) var(--sp-2);
  }
  .sec h3 { margin: 0; font-size: 15px; font-weight: 700; }
  .more {
    border: 0;
    background: transparent;
    color: var(--accent);
    font-size: 12px;
    font-weight: 600;
    padding: 0;
  }
  .compact { padding: 10px 12px; }
</style>
