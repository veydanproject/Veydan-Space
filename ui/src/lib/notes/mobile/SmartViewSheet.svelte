<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { api, formatError, type NoteFilter, type NoteNav, type SmartViewInput } from '$lib/notes/mobile/api';
  import { NAV_COLORS } from '$lib/core/mobile/nav-colors';
  import ColorSwatches from '$lib/core/ui/ColorSwatches.svelte';
  import { t } from '$lib/core/mobile/i18n';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';
  import Notice from '$lib/core/ui/Notice.svelte';
  import CustomSelect from '$lib/core/ui/CustomSelect.svelte';

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
    <input type="text" bind:value={name} placeholder={$t('notes_smart_name')} />
    <ColorSwatches bind:value={color} label={$t('notes_smart_name')} />

    {#if nav.all_workspaces.length}
      <div class="group">
        <span class="lbl">{$t('notes_filter_workspaces')}</span>
        <div class="chips">
          {#each nav.all_workspaces as ws (ws.id)}
            <button type="button" class="chip" class:active={bindings.includes(`workspace:${ws.id}`)} onclick={() => (bindings = toggle(bindings, `workspace:${ws.id}`))}>
              <span class="dot" style:background={ws.color}></span>{ws.name}
            </button>
          {/each}
          {#each nav.all_profiles as p (p.id)}
            <button type="button" class="chip" class:active={bindings.includes(`profile:${p.id}`)} onclick={() => (bindings = toggle(bindings, `profile:${p.id}`))}>
              {p.name}
            </button>
          {/each}
        </div>
      </div>
    {/if}

    <input type="text" bind:value={domain} placeholder={$t('notes_smart_domain')} />
    <CustomSelect
      options={[{ value: '', label: $t('notes_smart_any_folder') }, ...nav.folders.map((f) => ({ value: f.id, label: f.name }))]}
      bind:value={folderId}
      ariaLabel={$t('notes_col_folder')}
    />

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
            <button type="button" class="chip" class:active={tags.includes(tag.name)} onclick={() => (tags = toggle(tags, tag.name))}>
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

    <!-- The long form scrolls under the foot: its verb and its error always in sight. -->
    {#snippet footer()}
      {#if error}<Notice tone="danger">{error}</Notice>{/if}
      <button type="button" class="btn btn-ghost" onclick={onclose}>{$t('common_cancel')}</button>
      <button type="button" class="btn btn-primary" disabled={!name.trim() || saving} onclick={save}>
        {viewId ? $t('common_save') : $t('common_create')}
      </button>
    {/snippet}
</BottomSheet>

<style>
  .lbl { font-size: var(--fs-xs); font-weight: var(--fw-semibold); color: var(--text-3); }
  .lbl-row { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); }
  /* The common .chip, 32 high, --sp-2 apart: the zones of neighbours do not overlap. */
  .chips { display: flex; flex-wrap: wrap; gap: var(--sp-2); margin-top: var(--sp-2); }
  .chip { min-height: 32px; }
  .dot { width: 8px; height: 8px; border-radius: 50%; }
  .trow {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    min-height: var(--touch);
    font-size: var(--fs-body);
  }
  .days { width: 4rem; min-height: var(--touch); }
  .days-lbl { color: var(--text-2); font-size: var(--fs-sm); }
</style>
