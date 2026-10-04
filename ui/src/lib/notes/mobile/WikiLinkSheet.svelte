<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { api, type BindingSummary, type NoteListItem } from '$lib/notes/mobile/api';
  import { onKeyboard } from '$lib/core/mobile/keyboard';
  import { t } from '$lib/core/mobile/i18n';
  import { ENTITY_KINDS, isEntityKind, type EntityKind } from '$lib/core/bindings';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';

  interface Props {
    open: boolean;
    seed: string;
    notes: NoteListItem[];
    excludeId: string;
    onclose: () => void;
    /** `target` is a note title, or `kind:id` with `label` for an entity mention */
    onpick: (target: string, label?: string | null) => void;
  }

  let { open, seed, notes, excludeId, onclose, onpick }: Props = $props();

  const ICON: Record<EntityKind, string> = {
    workspace: 'layers',
    profile: 'globe',
    proxy: 'shield',
    ssh: 'terminal',
    totp: 'key',
    password: 'lock',
  };

  let query = $state('');
  let expanded = $state(false);
  let kb = $state(0);

  $effect(() => {
    if (!open) {
      expanded = false;
      return;
    }
    if (!expanded) query = seed;
  });

  $effect(() => {
    if (!open || expanded) return;
    return onKeyboard((n) => (kb = n));
  });

  const LIMIT = 60;

  const matches = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const live = notes.filter((n) => n.id !== excludeId && n.title);
    const list = q ? live.filter((n) => n.title.toLowerCase().includes(q)) : live;
    return list
      .sort((a, b) => {
        const as = a.title.toLowerCase().startsWith(q) ? 0 : 1;
        const bs = b.title.toLowerCase().startsWith(q) ? 0 : 1;
        return as - bs || b.updated_at.localeCompare(a.updated_at);
      })
      .slice(0, LIMIT);
  });

  const canCreate = $derived(
    query.trim().length > 0 && !matches.some((n) => n.title.toLowerCase() === query.trim().toLowerCase()),
  );

  // `kind:rest` switches the sheet to entity search
  const entityQuery = $derived.by((): { kind: EntityKind; rest: string } | null => {
    const i = query.indexOf(':');
    if (i < 0) return null;
    const kind = query.slice(0, i).trim().toLowerCase();
    return isEntityKind(kind) ? { kind, rest: query.slice(i + 1) } : null;
  });
  let entityHits = $state<BindingSummary[]>([]);
  let entityTimer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    const eq = entityQuery;
    if (entityTimer) clearTimeout(entityTimer);
    if (!eq) { entityHits = []; return; }
    entityTimer = setTimeout(async () => {
      const hits = await api.notes.entitySearch(eq.kind, eq.rest).catch(() => []);
      if (entityQuery?.kind === eq.kind && entityQuery.rest === eq.rest) entityHits = hits;
    }, 150);
  });

  function close() {
    expanded = false;
    onclose();
  }
</script>

{#if open && !expanded}
  <div class="bar" style:bottom="{kb}px" style:padding-bottom="{kb > 0 ? '8px' : 'calc(8px + var(--sab))'}">
    <button type="button" class="find" onclick={() => (expanded = true)}>
      <span class="ico"><Icon name="search" size={18} /></span>
      <span class="ph">{query.trim() || $t('notes_link_search')}</span>
    </button>
  </div>
{/if}

<BottomSheet open={open && expanded} title={$t('notes_links')} onclose={close}>
  <div class="find sheet">
    <span class="ico"><Icon name="search" size={18} /></span>
    <input
      type="search"
      bind:value={query}
      placeholder={$t('notes_link_search')}
      {@attach (el: HTMLInputElement) => { if (open && expanded) el.focus(); }}
    />
  </div>

  <div class="m-list">
    {#if entityQuery}
      {#each entityHits as s (s.binding)}
        <button type="button" class="m-row" onclick={() => onpick(s.binding, s.name)}>
          <Icon name={ICON[entityQuery.kind]} size={20} />
          <span class="m-row-label">{s.name}{#if s.subtitle}<span class="muted"> · {s.subtitle}</span>{/if}</span>
        </button>
      {/each}
      {#if entityHits.length === 0}
        <div class="m-row"><span class="m-row-label muted">{$t('notes_wiki_no_entities')}</span></div>
      {/if}
    {:else}
      {#each matches as n (n.id)}
        <button type="button" class="m-row" onclick={() => onpick(n.title)}>
          <Icon name="file-text" size={20} />
          <span class="m-row-label">{n.title}</span>
        </button>
      {/each}
      {#if canCreate}
        <button type="button" class="m-row" onclick={() => onpick(query.trim())}>
          <Icon name="plus" size={20} />
          <span class="m-row-label">{$t('notes_link_create', { title: query.trim() })}</span>
        </button>
      {/if}
      {#if matches.length === 0 && !canCreate}
        <div class="m-row"><span class="m-row-label muted">{$t('common_nothing_found')}</span></div>
      {/if}
    {/if}
  </div>
  {#if !query.trim()}
    <div class="hints">
      <span class="muted">{$t('notes_wiki_kind_hint')}</span>
      {#each ENTITY_KINDS as kind (kind)}
        <button type="button" class="hint" onclick={() => (query = `${kind}:`)}>
          <Icon name={ICON[kind]} size={12} />{kind}:
        </button>
      {/each}
    </div>
  {/if}
</BottomSheet>

<style>
  .bar {
    position: fixed;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 21;
    padding: 8px 12px;
    background: var(--m-nav);
    border-top: 1px solid var(--border);
  }
  .find {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    min-height: 44px;
    padding: 0 14px;
    border: 0;
    border-radius: 12px;
    background: var(--m-field);
    color: var(--text);
    font: inherit;
    text-align: left;
  }
  .find.sheet { flex-shrink: 0; }
  .find input {
    flex: 1;
    min-width: 0;
    min-height: 44px;
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
  }
  .find input:focus { box-shadow: none; }
  .ico { color: var(--text-3); display: inline-flex; flex-shrink: 0; }
  .ph { color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .muted { color: var(--text-3); }
  .hints { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding: var(--sp-3) var(--sp-2) 0; font-size: 13px; }
  .hint {
    display: inline-flex; align-items: center; gap: 4px;
    min-height: 32px; padding: 0 10px;
    border: 1px solid var(--border); border-radius: 999px;
    background: transparent; color: var(--text-2); font: inherit; font-size: 13px; font-family: var(--font-mono);
  }
</style>
