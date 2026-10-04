<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { ask } from '$lib/core/ui/confirm.svelte';
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import Icon from '$lib/core/Icon.svelte';
  import NotesBurger from '$lib/notes/mobile/NotesBurger.svelte';
  import { api, formatError, type NoteTag } from '$lib/notes/mobile/api';
  import { t } from '$lib/core/mobile/i18n';
  import { compareLabel } from '$lib/core/label-order';
  import { NAV_COLORS } from '$lib/core/mobile/nav-colors';
  import { longpress } from '$lib/core/mobile/longpress';
  import NavEditSheet from '$lib/notes/mobile/NavEditSheet.svelte';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';

  let tags = $state<NoteTag[]>([]);
  let query = $state('');
  let error = $state('');
  let menu = $state<NoteTag | null>(null);
  let edit = $state<{ id: string | null; title: string } | null>(null);
  let editName = $state('');
  let editColor = $state(NAV_COLORS[0]);
  let saving = $state(false);

  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return q ? tags.filter((x) => x.name.toLowerCase().includes(q)) : tags;
  });

  async function load() {
    try {
      tags = (await api.notes.tags()).sort((a, b) => compareLabel(a.name, b.name));
    } catch (e) {
      error = formatError(e);
    }
  }

  function openNew() {
    editName = query.trim();
    editColor = NAV_COLORS[0];
    edit = { id: null, title: $t('notes_tag_create') };
  }

  function openEdit() {
    if (!menu) return;
    editName = menu.name;
    editColor = menu.color || NAV_COLORS[0];
    edit = { id: menu.id, title: $t('notes_item_edit') };
    menu = null;
  }

  async function save(name: string, color: string) {
    if (!edit || saving) return;
    saving = true;
    error = '';
    try {
      if (edit.id) await api.notes.tagUpdate(edit.id, name, color);
      else await api.notes.tagCreate(name, color);
      edit = null;
      await load();
    } catch (e) {
      error = formatError(e);
    } finally {
      saving = false;
    }
  }

  async function remove() {
    if (!menu) return;
    if (!(await ask({ title: $t('notes_delete_item_confirm', { name: menu.name }) }))) return;
    const id = menu.id;
    menu = null;
    try {
      await api.notes.tagDelete(id);
      await load();
    } catch (e) {
      error = formatError(e);
    }
  }

  onMount(() => {
    load();
  });
</script>

<div class="m-page">
  <div class="m-header">
    <NotesBurger />
    <h1 class="m-title">{$t('notes_tags')}</h1>
    <button class="m-ibtn" onclick={openNew} aria-label={$t('notes_tag_create')}><Icon name="plus" size={24} /></button>
  </div>

  <div class="m-search">
    <div class="field">
      <Icon name="search" size={16} />
      <input type="search" bind:value={query} placeholder={$t('notes_tags_search')} />
    </div>
  </div>

  <div class="m-body">
  {#if error}<div class="m-error">{error}</div>{/if}

  {#if shown.length}
    <div class="m-list">
      {#each shown as tag (tag.id)}
        <button
          type="button"
          class="m-row"
          onclick={() => goto(`/notes?kind=tag&id=${encodeURIComponent(tag.name)}`)}
          {@attach longpress(() => (menu = tag))}
        >
          <span class="m-dot" style:background={tag.color}></span>
          <span class="m-row-label">{tag.name}</span>
          <span class="count">{tag.count}</span>
        </button>
      {/each}
    </div>
  {:else}
    <div class="m-empty">
      <Icon name="tag" size={40} />
      <p>{query.trim() ? $t('common_nothing_found') : $t('notes_areas_empty')}</p>
    </div>
  {/if}
  </div>
</div>

<BottomSheet open={!!menu} title={menu?.name ?? ''} onclose={() => (menu = null)}>
  <div class="m-list">
    <button type="button" class="m-row" onclick={openEdit}><Icon name="pencil" size={20} /><span class="m-row-label">{$t('notes_item_edit')}</span></button>
    <button type="button" class="m-row" onclick={remove}><Icon name="trash-2" size={20} /><span class="m-row-label danger">{$t('notes_tag_delete')}</span></button>
  </div>
</BottomSheet>

<NavEditSheet
  open={!!edit}
  title={edit?.title ?? ''}
  bind:name={editName}
  bind:color={editColor}
  placeholder={$t('notes_tag_name')}
  {saving}
  onclose={() => (edit = null)}
  onsave={save}
/>

<style>
  .danger { color: var(--danger-text); }
</style>
