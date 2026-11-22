<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { leave } from '$lib/core/ui/back';
  import Icon from '$lib/core/Icon.svelte';
  import { refocusSearch } from '$lib/core/ui/searchField';
  import Notice from '$lib/core/ui/Notice.svelte';
  import HubButton from '$lib/core/ui/HubButton.svelte';
  import Tabs from '$lib/core/ui/Tabs.svelte';
  import { api, formatError, type NoteSearchResult, type NoteTag } from '$lib/notes/mobile/api';
  import { locale, t } from '$lib/core/mobile/i18n';
  import NotesBurger from '$lib/notes/mobile/NotesBurger.svelte';
  import { fmtListDate } from '$lib/notes/mobile/notes-editor';

  type Scope = 'all' | 'notes' | 'tags';

  let query = $state('');
  let scope = $state<Scope>('all');
  const scopes = $derived<{ id: Scope; label: string }[]>([
    { id: 'all', label: $t('search_all') },
    { id: 'notes', label: $t('search_notes') },
    { id: 'tags', label: $t('search_tags') },
  ]);
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
    void leave('/');
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
    <HubButton />
    <NotesBurger />
    <h1 class="m-title">{$t('nav_search')}</h1>
  </div>
  <div class="m-find-row">
    <div class="search-field fill">
      <Icon name="search" size={16} />
      <input type="search" bind:value={query} oninput={onInput} placeholder={$t('search_placeholder')} />
      {#if query}
        <button type="button" class="search-clear" onclick={(e) => { query = ''; onInput(); refocusSearch(e); }} aria-label={$t('common_clear')}><Icon name="x" size={16} /></button>
      {/if}
    </div>
    <button type="button" class="m-link" onclick={cancel}>{$t('common_cancel')}</button>
  </div>

  <div class="m-tabs">
    <Tabs tabs={scopes} selected={scope} onpick={(id) => (scope = id)} label={$t('nav_search')} phone />
  </div>

  <div class="m-body">
  {#if error}<div class="err"><Notice tone="danger">{error}</Notice></div>{/if}

  {#if !q}
    <div class="empty-state"><span class="empty-icon"><Icon name="search" size={22} /></span><p>{$t('search_hint')}</p></div>
  {:else if empty}
    <div class="empty-state"><span class="empty-icon"><Icon name="search" size={22} /></span><p>{$t('common_nothing_found')}</p></div>
  {:else}
    {#if scope !== 'tags' && notes.length}
      <div class="sec">
        <h3>{$t('search_notes')} ({notes.length})</h3>
        {#if !showAll && scope === 'all' && notes.length > 3}
          <button type="button" class="more" onclick={() => (showAll = true)}>{$t('search_show_all')}</button>
        {/if}
      </div>
      <div class="hits divided">
        {#each shownNotes as n (n.id)}
          <a class="list-row hit" href="/notes/{n.id}">
            <span class="list-row-body">
              <span class="list-row-top">
                <span class="list-row-name">{@render hl(n.title || $t('notes_untitled'))}</span>
                <span class="list-row-time">{fmtListDate(n.updated_at, $locale)}</span>
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
          <a class="list-row dense" href="/notes?kind=tag&id={encodeURIComponent(tag.name)}">
            <span class="m-folder tinted-text" style:--chip={tag.color || undefined}><Icon name="tag" size={16} /></span>
            <span class="list-row-label">{@render hl(tag.name)}</span>
            <span class="count-pill lg quiet">{tag.count}</span>
          </a>
        {/each}
      </div>
    {/if}

    {#if scope !== 'tags' && textHits.length}
      <div class="sec"><h3>{$t('search_text_matches')}</h3></div>
      <div class="hits divided">
        {#each textHits as m (m.id)}
          <a class="list-row hit" href="/notes/{m.id}">
            <span class="list-row-body">
              <span class="list-row-top">
                <span class="list-row-name">{m.title || $t('notes_untitled')}</span>
                <span class="list-row-time">{fmtListDate(m.updated_at, $locale)}</span>
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
  .err { margin-bottom: var(--sp-3); }
  .sec {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--sp-4) var(--sp-1) var(--sp-2);
  }
  .sec h3 { margin: 0; font-size: var(--fs-body); font-weight: var(--fw-bold); }
  .more {
    border: 0;
    background: transparent;
    color: var(--accent-text);
    font-size: var(--fs-xs);
    font-weight: var(--fw-semibold);
    padding: 0;
  }
  /* Rows of the list as the notes' (NoteRow): the common row of a record (base.css .list-row, 64), flat on the
     page with a hairline between them; the name, the date, the preview of two lines under them. */
  .hits { display: flex; flex-direction: column; gap: var(--sp-1); --rows-gap: var(--sp-1); }
  .hit .list-row-body { gap: 4px; }
  .preview {
    font-size: var(--fs-sm); color: var(--text-2);
    display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;
  }
</style>
