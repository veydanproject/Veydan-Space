<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import type { NoteListItem } from '$lib/notes/types';
  import Icon from '$lib/core/Icon.svelte';
  import Suggest, { type SuggestItem } from '$lib/core/ui/Suggest.svelte';
  import { t } from '$lib/core/i18n';
  import { binding, isEntityKind, type EntityKind } from '$lib/core/bindings';
  import { entityKind, entityKinds, searchEntities } from '$lib/notes/context';

  interface Props {
    query: string;
    notes: NoteListItem[];
    excludeId?: string | null;
    /** Highlighted row; moved by arrow keys via `handleKeydown` */
    index?: number;
    /** The editor: the list's floor is its dialog's footer. */
    anchor: HTMLElement | null | undefined;
    /** The line of the caret on the screen: the list goes under it. */
    caret: () => { left: number; top: number; bottom: number } | null;
    /** `target` is a note title, or `kind:id` with `label` for an entity mention */
    onpick: (target: string, label?: string | null) => void;
  }

  let { query, notes, excludeId = null, index = $bindable(0), anchor, caret, onpick }: Props = $props();

  let list = $state<Suggest | null>(null);

  interface Row extends SuggestItem {
    target: string;
    label: string | null;
  }

  /** `kind:rest` when the query starts with an entity kind prefix. */
  const entityQuery = $derived.by((): { kind: EntityKind; rest: string } | null => {
    const i = query.indexOf(':');
    if (i < 0) return null;
    const kind = query.slice(0, i).trim().toLowerCase();
    return isEntityKind(kind) ? { kind, rest: query.slice(i + 1) } : null;
  });

  const rows = $derived.by((): Row[] => {
    if (entityQuery) {
      const def = entityKind(entityQuery.kind);
      if (!def) return [];
      return searchEntities(entityQuery.kind, entityQuery.rest, Infinity).map((e) => {
        const target = binding(entityQuery.kind, e.id);
        return { key: target, target, label: e.name, icon: def.icon, text: e.name };
      });
    }
    const q = query.trim().toLowerCase();
    const live = notes.filter((n) => n.id !== excludeId && !n.deleted);
    const list = q ? live.filter((n) => n.title.toLowerCase().includes(q)) : live;
    const out: Row[] = list
      .sort((a, b) => {
        const as = a.title.toLowerCase().startsWith(q) ? 0 : 1;
        const bs = b.title.toLowerCase().startsWith(q) ? 0 : 1;
        return as - bs || b.updated_at.localeCompare(a.updated_at);
      })
      .map((n) => ({ key: n.id, target: n.title, label: null, icon: 'notes', text: n.title }));
    if (q && !out.some((r) => r.text.toLowerCase() === q)) {
      out.push({ key: 'create', target: query.trim(), label: null, icon: 'plus', text: $t('note_link_create', { title: query.trim() }), create: true });
    }
    return out;
  });

  /** Kind prefixes offered while nothing has been typed yet. */
  const kindHints = $derived(query.trim().length === 0 ? entityKinds() : []);

  function pick(row: SuggestItem) {
    const r = row as Row;
    onpick(r.target, r.label);
  }

  /** Arrow navigation and Enter/Tab pick; returns true when the key was consumed. */
  export function handleKeydown(e: KeyboardEvent): boolean {
    if (e.key === 'Tab' && rows[index]) { e.preventDefault(); pick(rows[index]); return true; }
    return list?.keydown(e) ?? false;
  }
</script>

<Suggest
  bind:this={list}
  bind:active={index}
  first={0}
  items={rows}
  {anchor}
  {caret}
  empty={entityQuery ? $t('common_nothing_found') : $t('note_link_type_to_search')}
  onpick={pick}
>
  {#snippet foot()}
    {#if kindHints.length > 0}
      <div class="hints">
        {#each kindHints as def (def.kind)}
          <span class="hint"><Icon name={def.icon} size={10} />{def.kind}:</span>
        {/each}
      </div>
    {/if}
  {/snippet}
</Suggest>

<style>
  .hints { display: flex; flex-wrap: wrap; flex-shrink: 0; gap: 0.3rem; margin-top: 4px; padding: 0.3rem 0.5rem 0.15rem; border-top: 1px solid var(--border); }
  .hint { display: inline-flex; align-items: center; gap: 0.2rem; font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-3); }
</style>
