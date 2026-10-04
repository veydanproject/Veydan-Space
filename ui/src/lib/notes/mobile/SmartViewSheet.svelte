<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { api, formatError, type NoteFilter, type NoteNav, type SmartViewInput } from '$lib/notes/mobile/api';
  import { NAV_COLORS } from '$lib/core/mobile/nav-colors';
  import { t } from '$lib/core/mobile/i18n';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';

  interface Props {
    open: boolean;
    viewId: string | null;
    nav: NoteNav;
    onclose: () => void;
    onsaved: (id: string) => void;
  }

  let { open, viewId, nav, onclose, onsaved }: Props = $props();

  let name = $state('');
  let color = $state(NAV_COLORS[0]);
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
  let error = $state('');

  $effect(() => {
    if (!open) return;
    error = '';
    if (!viewId) {
      name = '';
      color = NAV_COLORS[0];
      bindings = [];
      domain = '';
      folderId = '';
      tags = [];
      tagsMatchAll = false;
      pinnedOnly = false;
      updatedDays = '';
      hasAttachments = false;
      hasOpenTasks = false;
      return;
    }
    api.notes.smartViewGet(viewId).then((view) => {
      const c: NoteFilter = view.conditions ?? {};
      name = view.name;
      color = view.color || NAV_COLORS[0];
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
    }).catch((e) => { error = formatError(e); });
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
      if (tagsMatchAll) c.tags_all = tags;
      else c.tags_any = tags;
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
    error = '';
    const input: SmartViewInput = { name: name.trim(), color, conditions: buildConditions() };
    try {
      const view = viewId
        ? await api.notes.smartViewUpdate(viewId, input)
        : await api.notes.smartViewCreate(input);
      onsaved(view.id);
      onclose();
    } catch (e) {
      error = formatError(e);
    } finally {
      saving = false;
    }
  }
</script>

<BottomSheet {open} title={viewId ? $t('notes_smart_edit') : $t('notes_smart_new')} {onclose}>
    {#if error}<div class="m-error">{error}</div>{/if}
    <input class="input" type="text" bind:value={name} placeholder={$t('notes_smart_name')} />
    <div class="swatches">
      {#each NAV_COLORS as c}
        <button type="button" class="swatch" class:on={color === c} style:background={c} aria-label={c} onclick={() => (color = c)}></button>
      {/each}
    </div>

    {#if nav.all_workspaces.length}
      <div class="group">
        <span class="lbl">{$t('notes_filter_workspaces')}</span>
        <div class="chips">
          {#each nav.all_workspaces as ws (ws.id)}
            <button type="button" class="chip" class:on={bindings.includes(`workspace:${ws.id}`)} onclick={() => (bindings = toggle(bindings, `workspace:${ws.id}`))}>
              <span class="dot" style:background={ws.color}></span>{ws.name}
            </button>
          {/each}
          {#each nav.all_profiles as p (p.id)}
            <button type="button" class="chip" class:on={bindings.includes(`profile:${p.id}`)} onclick={() => (bindings = toggle(bindings, `profile:${p.id}`))}>
              {p.name}
            </button>
          {/each}
        </div>
      </div>
    {/if}

    <input class="input" type="text" bind:value={domain} placeholder={$t('notes_smart_domain')} />
    <select class="input" bind:value={folderId}>
      <option value="">{$t('notes_smart_any_folder')}</option>
      {#each nav.folders as f (f.id)}
        <option value={f.id}>{f.name}</option>
      {/each}
    </select>

    {#if nav.tags.length}
      <div class="group">
        <div class="lbl-row">
          <span class="lbl">{$t('notes_areas_title')}</span>
          {#if tags.length > 1}
            <div class="trow">
              <span>{$t('notes_smart_tags_all')}</span>
              <button type="button" class="toggle" class:on={tagsMatchAll} onclick={() => (tagsMatchAll = !tagsMatchAll)} aria-pressed={tagsMatchAll}></button>
            </div>
          {/if}
        </div>
        <div class="chips">
          {#each nav.tags as tag (tag.id)}
            <button type="button" class="chip" class:on={tags.includes(tag.name)} onclick={() => (tags = toggle(tags, tag.name))}>
              <span class="dot" style:background={tag.color}></span>{tag.name}
            </button>
          {/each}
        </div>
      </div>
    {/if}

    <div class="trow">
      <span>{$t('notes_filter_pinned')}</span>
      <button type="button" class="toggle" class:on={pinnedOnly} onclick={() => (pinnedOnly = !pinnedOnly)} aria-pressed={pinnedOnly}></button>
    </div>
    <div class="trow">
      <span>{$t('notes_smart_has_attachments')}</span>
      <button type="button" class="toggle" class:on={hasAttachments} onclick={() => (hasAttachments = !hasAttachments)} aria-pressed={hasAttachments}></button>
    </div>
    <div class="trow">
      <span>{$t('notes_smart_has_open_tasks')}</span>
      <button type="button" class="toggle" class:on={hasOpenTasks} onclick={() => (hasOpenTasks = !hasOpenTasks)} aria-pressed={hasOpenTasks}></button>
    </div>
    <div class="trow">
      <span>{$t('notes_smart_updated_within')}</span>
      <input class="days" type="number" min="1" bind:value={updatedDays} placeholder="—" />
      <span class="days-lbl">{$t('notes_smart_days')}</span>
    </div>

    <button type="button" class="m-btn-grad" disabled={!name.trim() || saving} onclick={save}>
      {viewId ? $t('notes_smart_save') : $t('notes_btn_create')}
    </button>
</BottomSheet>

<style>
  .input { width: 100%; border: 0; border-radius: 12px; background: var(--m-field); padding: 0 14px; }
  .swatches { display: flex; gap: 12px; flex-wrap: wrap; padding: 3px; }
  .swatch {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    border: 3px solid transparent;
    padding: 0;
    box-shadow: 0 0 0 2px var(--m-card-border);
  }
  .swatch.on { border-color: var(--m-sheet); box-shadow: 0 0 0 2px var(--text); }
  .lbl { font-size: var(--fs-xs); font-weight: 600; color: var(--text-3); text-transform: uppercase; }
  .lbl-row { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 6px; }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-radius: 999px;
    border: 0;
    background: var(--m-seg);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-sm);
    font-weight: 600;
  }
  .chip.on { background: var(--accent); color: #fff; }
  .dot { width: 8px; height: 8px; border-radius: 50%; }
  .trow {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    min-height: var(--touch);
    font-size: 15px;
  }
  .days { width: 4rem; min-height: var(--touch); }
  .days-lbl { color: var(--text-2); font-size: var(--fs-sm); }
</style>
