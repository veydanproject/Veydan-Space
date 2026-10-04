<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import type { NoteFilter, NoteSmartView, SmartViewInput } from '$lib/notes/types';
  import { notesStore } from '$lib/notes/store/notes.svelte';
  import { directory } from '$lib/core/directory';
  import { t } from '$lib/core/i18n';

  const COLORS = [
    '#8b7bff', '#60a5fa', '#2dd4bf', '#f472b6',
    '#f5c451', '#34d399', '#f26d6d', '#f97316',
  ];

  interface Props {
    open: boolean;
    /** Existing view to edit; null = create */
    view: NoteSmartView | null;
    onclose: () => void;
  }

  let { open, view, onclose }: Props = $props();

  let name = $state('');
  let color = $state(COLORS[0]);
  let bindings = $state<string[]>([]);
  let domain = $state('');
  let folderId = $state('');
  let tags = $state<string[]>([]);
  let tagsMatchAll = $state(false);
  let pinnedOnly = $state(false);
  let updatedDays = $state('');
  let hasAttachments = $state(false);
  let hasOpenTasks = $state(false);
  let saving = $state(false);
  /** Profiles are many: they have a group of their own with a search. */
  let profileQuery = $state('');
  const profiles = $derived.by(() => {
    const q = profileQuery.trim().toLowerCase();
    const all = directory.list('profile');
    // The chosen ones stay visible whatever the search says.
    return q ? all.filter((p) => p.name.toLowerCase().includes(q) || bindings.includes(`profile:${p.id}`)) : all;
  });

  // Load form from the view whenever the dialog opens
  $effect(() => {
    if (!open) return;
    const c: NoteFilter = view?.conditions ?? {};
    name = view?.name ?? '';
    color = view?.color ?? COLORS[0];
    const any = c.bindings_any ?? [];
    bindings = any.filter((b) => !b.startsWith('domain:'));
    domain = any.find((b) => b.startsWith('domain:'))?.slice('domain:'.length) ?? '';
    folderId = c.folder_id ?? '';
    tags = c.tags_all ?? c.tags_any ?? [];
    tagsMatchAll = !!c.tags_all;
    pinnedOnly = c.pinned === true;
    updatedDays = c.updated_within_days ? String(c.updated_within_days) : '';
    hasAttachments = c.has_attachments === true;
    hasOpenTasks = c.has_open_tasks === true;
  });

  function toggle(list: string[], value: string): string[] {
    return list.includes(value) ? list.filter((v) => v !== value) : [...list, value];
  }

  function buildConditions(): NoteFilter {
    const c: NoteFilter = {};
    const any = [...bindings];
    if (domain.trim()) any.push(`domain:${domain.trim().toLowerCase()}`);
    if (any.length) c.bindings_any = any;
    if (folderId) c.folder_id = folderId;
    if (tags.length) {
      if (tagsMatchAll) c.tags_all = tags; else c.tags_any = tags;
    }
    if (pinnedOnly) c.pinned = true;
    const days = parseInt(updatedDays, 10);
    if (Number.isFinite(days) && days > 0) c.updated_within_days = days;
    if (hasAttachments) c.has_attachments = true;
    if (hasOpenTasks) c.has_open_tasks = true;
    return c;
  }

  async function save() {
    if (!name.trim() || saving) return;
    saving = true;
    const input: SmartViewInput = { name: name.trim(), color, conditions: buildConditions() };
    try {
      if (view) await notesStore.updateSmartView(view.id, input);
      else await notesStore.createSmartView(input);
      onclose();
    } finally {
      saving = false;
    }
  }
</script>

<Dialog {open} width="380px" title={view ? $t('notes_smart_edit') : $t('notes_smart_new')} {onclose}>
  <div class="body">
    <input
      class="input"
      type="text"
      bind:value={name}
      placeholder={$t('notes_smart_name')}
      onkeydown={(e) => { if (e.key === 'Enter') void save(); }}
    />

    <div class="color-row">
      {#each COLORS as c}
        <button class="swatch" class:active={color === c} style="background:{c}" aria-label={c} onclick={() => (color = c)}></button>
      {/each}
    </div>

    {#if directory.list('workspace').length > 0}
      <div class="group">
        <span class="label">{$t('notes_filter_workspaces')}</span>
        <div class="chips">
          {#each directory.list('workspace') as ws (ws.id)}
            <button class="chip" class:on={bindings.includes(`workspace:${ws.id}`)} onclick={() => (bindings = toggle(bindings, `workspace:${ws.id}`))}>
              <span class="dot" style="background:{ws.color}"></span>{ws.name}
            </button>
          {/each}
        </div>
      </div>
    {/if}

    {#if directory.list('profile').length > 0}
      <div class="group">
        <div class="label-row">
          <span class="label">{$t('profiles_title')}</span>
          {#if directory.list('profile').length > 12}
            <input class="input profile-search" type="search" bind:value={profileQuery} placeholder={$t('notes_smart_profile_search')} />
          {/if}
        </div>
        <div class="chips chips-scroll">
          {#each profiles as p (p.id)}
            <button class="chip" class:on={bindings.includes(`profile:${p.id}`)} onclick={() => (bindings = toggle(bindings, `profile:${p.id}`))}>
              {p.name}
            </button>
          {/each}
        </div>
      </div>
    {/if}

    <div class="row">
      <input class="input" type="text" bind:value={domain} placeholder={$t('notes_smart_domain')} />
      <select class="input" bind:value={folderId}>
        <option value="">{$t('notes_smart_any_folder')}</option>
        {#each notesStore.folders as f (f.id)}
          <option value={f.id}>{f.name}</option>
        {/each}
      </select>
    </div>

    {#if notesStore.allTags.length > 0}
      <div class="group">
        <div class="label-row">
          <span class="label">{$t('notes_areas_title')}</span>
          {#if tags.length > 1}
            <label class="check small"><input type="checkbox" bind:checked={tagsMatchAll} /> {$t('notes_smart_tags_all')}</label>
          {/if}
        </div>
        <div class="chips">
          {#each notesStore.allTags as tag (tag.id)}
            <button class="chip" class:on={tags.includes(tag.name)} onclick={() => (tags = toggle(tags, tag.name))}>
              <span class="dot" style="background:{tag.color}"></span>{tag.name}
            </button>
          {/each}
        </div>
      </div>
    {/if}

    <div class="checks">
      <label class="check"><input type="checkbox" bind:checked={pinnedOnly} /> {$t('notes_filter_pinned')}</label>
      <label class="check"><input type="checkbox" bind:checked={hasAttachments} /> {$t('notes_smart_has_attachments')}</label>
      <label class="check"><input type="checkbox" bind:checked={hasOpenTasks} /> {$t('notes_smart_has_open_tasks')}</label>
      <label class="check">
        {$t('notes_smart_updated_within')}
        <input class="input days" type="number" min="1" bind:value={updatedDays} placeholder="—" />
        {$t('notes_smart_days')}
      </label>
    </div>

    <div class="actions">
      <button class="btn btn-ghost btn-sm" onclick={onclose}>{$t('notes_btn_cancel')}</button>
      <button class="btn btn-primary btn-sm" onclick={save} disabled={!name.trim() || saving}>
        {view ? $t('notes_smart_save') : $t('notes_btn_create')}
      </button>
    </div>
  </div>
</Dialog>

<style>
  .body { display: flex; flex-direction: column; gap: 0.6rem; }
  .input {
    width: 100%;
    min-width: 0;
    padding: 0.4rem 0.55rem;
    font-size: var(--fs-sm);
    border: 1px solid var(--border);
    border-radius: 6px;
    /* background-color only: a select keeps the chevron of base.css */
    background-color: var(--bg-2);
    color: var(--text);
  }
  .row { display: grid; grid-template-columns: 1fr 1fr; gap: 0.4rem; }
  .color-row { display: flex; gap: 0.3rem; flex-wrap: wrap; }
  .swatch {
    width: 18px; height: 18px; border-radius: 50%;
    border: 2px solid transparent; cursor: pointer; padding: 0;
  }
  .swatch.active { border-color: var(--text); }
  .group { display: flex; flex-direction: column; gap: 0.3rem; }
  .label-row { display: flex; justify-content: space-between; align-items: center; }
  .label {
    font-size: var(--fs-2xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-3);
    font-weight: 600;
  }
  .chips { display: flex; flex-wrap: wrap; gap: 0.3rem; }
  /* A long list of profiles scrolls inside its group: the dialog keeps the window's height. */
  .chips-scroll { max-height: 120px; overflow-y: auto; align-content: flex-start; }
  .profile-search { width: 180px; padding-top: 0.25rem; padding-bottom: 0.25rem; font-size: var(--fs-xs); }
  .chip {
    display: inline-flex; align-items: center; gap: 0.3rem;
    padding: 0.2rem 0.5rem;
    font-size: var(--fs-xs);
    border: 1px solid var(--border);
    border-radius: 999px;
    background: transparent;
    color: var(--text-2);
    cursor: pointer;
  }
  .chip.on { border-color: var(--accent); color: var(--text); background: color-mix(in srgb, var(--accent) 15%, transparent); }
  .dot { width: 7px; height: 7px; border-radius: 50%; display: inline-block; }
  .checks { display: flex; flex-direction: column; gap: 0.3rem; }
  .check { display: flex; align-items: center; gap: 0.4rem; font-size: var(--fs-sm); color: var(--text-2); }
  /* base.css gives every input width:100% + padding; keep the box inline */
  .check input[type="checkbox"] { margin: 0; flex-shrink: 0; }
  .check.small { font-size: var(--fs-xs); }
  .days { width: 4.5rem; padding: 0.2rem 0.4rem; }
  .actions { display: flex; justify-content: flex-end; gap: 0.4rem; margin-top: 0.2rem; }
</style>
