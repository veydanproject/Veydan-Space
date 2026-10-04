<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import type { NavChild } from '$lib/notes/mobile/api';
  import { t } from '$lib/core/mobile/i18n';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';

  interface Props {
    open: boolean;
    folders: NavChild[];
    /** Folder the note is in now; null for none. */
    current: string | null;
    busy?: boolean;
    onclose: () => void;
    onmove: (folderId: string | null) => void;
  }

  let { open, folders, current, busy = false, onclose, onmove }: Props = $props();

  let query = $state('');
  let picked = $state<string | null>(null);
  let collapsed = $state<Set<string>>(new Set());

  $effect(() => {
    if (open) {
      picked = current;
      query = '';
    }
  });

  type Node = { item: NavChild; children: Node[] };
  const tree = $derived.by((): Node[] => {
    const map = new Map<string, Node>();
    for (const f of folders) map.set(f.id, { item: f, children: [] });
    const roots: Node[] = [];
    for (const n of map.values()) {
      const pid = n.item.parent_id;
      if (pid && map.has(pid)) map.get(pid)!.children.push(n);
      else roots.push(n);
    }
    return roots;
  });

  const q = $derived(query.trim().toLowerCase());
  const flatMatches = $derived(q ? folders.filter((f) => f.name.toLowerCase().includes(q)) : []);

  function toggle(id: string) {
    const next = new Set(collapsed);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    collapsed = next;
  }
</script>

{#snippet row(f: NavChild, depth: number, hasKids: boolean)}
  <div class="row" style:padding-left="{depth * 24}px">
    {#if hasKids}
      <button type="button" class="caret" onclick={() => toggle(f.id)} aria-label={f.name}>
        <Icon name={collapsed.has(f.id) ? 'chevron-right' : 'chevron-down'} size={14} />
      </button>
    {:else}
      <span class="caret"></span>
    {/if}
    <button type="button" class="pick" class:on={picked === f.id} onclick={() => (picked = f.id)}>
      <span class="m-doc" style:--c={f.color}><Icon name="folder" size={16} /></span>
      <span class="name" class:bold={depth === 0}>{f.name}</span>
      <span class="radio" class:on={picked === f.id}></span>
    </button>
  </div>
{/snippet}

{#snippet node(n: Node, depth: number)}
  {@render row(n.item, depth, n.children.length > 0)}
  {#if n.children.length && !collapsed.has(n.item.id)}
    {#each n.children as c (c.item.id)}{@render node(c, depth + 1)}{/each}
  {/if}
{/snippet}

<BottomSheet {open} title={$t('notes_move')} {onclose}>
  <div class="m-search">
    <div class="field">
      <Icon name="search" size={16} />
      <input type="search" bind:value={query} placeholder={$t('notes_move_search')} />
    </div>
  </div>

  <div class="tree">
    <div class="row">
      <span class="caret"></span>
      <button type="button" class="pick" class:on={picked === null} onclick={() => (picked = null)}>
        <span class="m-doc muted"><Icon name="folder" size={16} /></span>
        <span class="name">{$t('notes_move_none')}</span>
        <span class="radio" class:on={picked === null}></span>
      </button>
    </div>
    {#if q}
      {#each flatMatches as f (f.id)}{@render row(f, 0, false)}{/each}
    {:else}
      {#each tree as n (n.item.id)}{@render node(n, 0)}{/each}
    {/if}
  </div>

  <button type="button" class="m-btn-grad" disabled={busy || picked === current} onclick={() => onmove(picked)}>
    {$t('notes_move_btn')}
  </button>
</BottomSheet>

<style>
  .tree { flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; }
  .row { display: flex; align-items: center; }
  .caret {
    width: 28px;
    height: 44px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text-3);
    flex-shrink: 0;
  }
  .pick {
    flex: 1;
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-height: 48px;
    padding: 0 var(--sp-2);
    border: 0;
    border-radius: 12px;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 15px;
    text-align: left;
  }
  .pick.on { background: var(--accent-bg); }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .name.bold { font-weight: 700; }
  .radio {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    border: 2px solid var(--border-2);
    flex-shrink: 0;
    position: relative;
  }
  .radio.on { border-color: var(--accent); }
  .radio.on::after {
    content: '';
    position: absolute;
    inset: 3px;
    border-radius: 50%;
    background: var(--accent);
  }
</style>
