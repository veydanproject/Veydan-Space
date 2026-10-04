<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import type { NoteTag, NoteTagInfo, NoteFolder } from '$lib/notes/types';
  import { api } from '$lib/notes/api';
  import Icon from '$lib/core/Icon.svelte';
  import { t, type TranslationKey } from '$lib/core/i18n';
  import ChipMark from '$lib/core/ui/ChipMark.svelte';
  import { binding, type EntityKind } from '$lib/core/bindings';
  import { entityKind, entityKinds, ensureEntitiesLoaded, searchEntities, type EntitySummary } from '$lib/notes/context';
  import { labelPickerText } from '$lib/notes/label-kinds';

  interface ContextChip {
    kind: string;
    label: string;
    color: string;
    onremove?: () => void;
  }

  interface Props {
    selectedTags: NoteTagInfo[];
    allTags: NoteTag[];
    onchange: (tagNames: string[]) => void;
    contextChips?: ContextChip[];
    folders?: NoteFolder[];
    activeFolderIds?: string[];
    onaddFolder?: (folderId: string) => void;
    activeBindings?: string[];
    onaddBinding?: (binding: string) => void;
  }

  let { selectedTags, allTags, onchange, contextChips = [], folders = [], activeFolderIds = [], onaddFolder, activeBindings = [], onaddBinding }: Props = $props();

  const TAG_COLORS = [
    '#8b7bff', '#60a5fa', '#2dd4bf', '#f472b6',
    '#f5c451', '#34d399', '#f26d6d', '#f97316',
  ];

  let open = $state(false);
  let inputValue = $state('');
  let selectedColor = $state(TAG_COLORS[0]);
  let inputEl: HTMLInputElement | null = $state(null);
  let wrapEl: HTMLDivElement | null = $state(null);

  const selectedNames = $derived(new Set(selectedTags.map((t) => t.name)));

  const suggestions = $derived(
    allTags
      .filter((t) => !selectedNames.has(t.name) && t.name.toLowerCase().includes(inputValue.toLowerCase()))
      .slice(0, 6)
  );

  const folderSuggestions = $derived(
    inputValue.trim().length > 0
      ? folders
          .filter(f => !activeFolderIds.includes(f.id) && f.name.toLowerCase().includes(inputValue.toLowerCase()))
          .slice(0, 4)
      : []
  );

  /** Veydan entities matching the input. With an empty field, offer TOTP and passwords to bind. */
  const entitySuggestions = $derived.by(() => {
    const q = inputValue.trim();
    const kinds = (q ? entityKinds().map((d) => d.kind) : ['totp', 'password']) as EntityKind[];
    const max = q ? 3 : 6;
    const out: { binding: string; kindLabel: TranslationKey; entity: EntitySummary }[] = [];
    for (const kind of kinds) {
      for (const entity of searchEntities(kind, q, max)) {
        const b = binding(kind, entity.id);
        const def = entityKind(kind);
        if (def && !activeBindings.includes(b)) out.push({ binding: b, kindLabel: def.label, entity });
      }
    }
    return out;
  });

  /** Placeholder and hint name only the kinds this product has entities of. */
  const pickerText = $derived(labelPickerText(entityKinds().filter((d) => d.list().length > 0).map((d) => d.kind), $t));

  const isNew = $derived(
    inputValue.trim().length > 0 && !allTags.some((t) => t.name === inputValue.trim())
  );

  /** The popup opens leftwards when the + is too close to the right edge of the editor (which clips it). */
  let alignRight = $state(false);
  const POPUP_W = 260;

  function openPopup(e: MouseEvent) {
    const btn = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const edge = (wrapEl?.closest('.editor') ?? document.body).getBoundingClientRect().right;
    alignRight = btn.left + POPUP_W > edge;
    open = true;
    inputValue = '';
    selectedColor = TAG_COLORS[0];
    void ensureEntitiesLoaded();
    setTimeout(() => inputEl?.focus(), 50);
  }

  function close() {
    open = false;
    inputValue = '';
  }

  async function addTag(name: string, color?: string) {
    const trimmed = name.trim();
    if (!trimmed || selectedNames.has(trimmed)) { close(); return; }

    const existing = allTags.find((t) => t.name === trimmed);
    if (!existing && color) {
      await api.notes.tagCreate(trimmed, color);
    }

    onchange([...selectedNames, trimmed]);
    close();
  }

  function removeTag(name: string) {
    onchange([...selectedNames].filter((n) => n !== name));
  }

  function onKeydown(e: KeyboardEvent) {
    if ((e.key === 'Enter' || e.key === ',') && inputValue.trim()) {
      e.preventDefault();
      void addTag(inputValue, selectedColor);
    }
    if (e.key === 'Escape') close();
  }

  function onOutsideClick(e: MouseEvent) {
    if (wrapEl && !wrapEl.contains(e.target as Node)) close();
  }
</script>

<svelte:window onclick={onOutsideClick} />

<div class="tags-outer" bind:this={wrapEl}>
  <div class="tags-wrap">
    {#each contextChips as chip}
      <span class="chip tinted" style:--chip={chip.color}>
        <ChipMark kind={chip.kind} />
        {chip.label}
        {#if chip.onremove}
          <button class="chip-x" onclick={chip.onremove} aria-label={$t('notes_label_remove', { name: chip.label })}>×</button>
        {/if}
      </span>
    {/each}

    {#each selectedTags as tag (tag.id)}
      <span class="chip tinted" style:--chip={tag.color}>
        <ChipMark kind="tag" />
        {tag.name}
        <button class="chip-x" onclick={() => removeTag(tag.name)} aria-label={$t('notes_label_remove', { name: tag.name })}>×</button>
      </span>
    {/each}
  </div>

  <div class="add-wrap">
    <button class="add-btn" onclick={openPopup} title={$t('notes_label_title')}>
      <Icon name="plus" size={12} />
    </button>

    {#if open}
      <div class="popup" class:align-right={alignRight}>
        <input
          bind:this={inputEl}
          bind:value={inputValue}
          type="text"
          class="popup-input"
          placeholder={pickerText.placeholder}
          onkeydown={onKeydown}
        />
        {#if !inputValue.trim() && entitySuggestions.length === 0}
          <div class="hint">{pickerText.hint}</div>
        {/if}

        {#if suggestions.length > 0 || folderSuggestions.length > 0 || entitySuggestions.length > 0}
          <div class="suggestions">
            {#each suggestions as s (s.id)}
              <button class="sug-item" onmousedown={(e) => { e.preventDefault(); void addTag(s.name); }}>
                <span class="sug-dot" style="background:{s.color}"></span>
                {s.name}
              </button>
            {/each}
            {#each folderSuggestions as f (f.id)}
              <button class="sug-item" onmousedown={(e) => { e.preventDefault(); onaddFolder?.(f.id); close(); }}>
                <span class="sug-dot" style="background:{f.color}"></span>
                {f.name}
                <span class="sug-folder-label">{$t('ctx_kind_note_folder')}</span>
              </button>
            {/each}
            {#each entitySuggestions as s (s.binding)}
              <button class="sug-item" onmousedown={(e) => { e.preventDefault(); onaddBinding?.(s.binding); close(); }}>
                <span class="sug-dot" style="background:{s.entity.color}"></span>
                {s.entity.name}
                <span class="sug-folder-label">{$t(s.kindLabel)}</span>
              </button>
            {/each}
          </div>
        {/if}

        {#if isNew}
          <div class="color-row">
            {#each TAG_COLORS as c}
              <button
                class="color-swatch"
                class:active={selectedColor === c}
                style="background:{c}"
                aria-label={$t('notes_color', { c })}
                onclick={() => (selectedColor = c)}
              ></button>
            {/each}
            <label
              class="color-swatch color-swatch-custom"
              class:active={!TAG_COLORS.includes(selectedColor)}
              title={$t('notes_color_custom')}
            >
              <input type="color" bind:value={selectedColor} />
              {#if !TAG_COLORS.includes(selectedColor)}
                <span class="custom-dot" style="background:{selectedColor}"></span>
              {/if}
            </label>
          </div>
          <button class="btn btn-primary btn-sm create-btn" onclick={() => addTag(inputValue, selectedColor)}>
            {$t('notes_tag_add')}
          </button>
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  /* The chips have a row of their own under the title (NoteEditor): they wrap, nothing is cut */
  .tags-outer {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.3rem;
    flex: 1;
    min-width: 0;
  }

  .tags-wrap { display: contents; }

  .chip {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.2rem;
    font-size: var(--fs-2xs);
    line-height: 1;
    padding: 0.2rem 0.45rem;
    border-radius: 999px;
    border-width: 1px;
    border-style: solid;
    font-weight: 500;
    white-space: nowrap;
  }

  .chip-x {
    background: none;
    border: none;
    cursor: pointer;
    padding: 0;
    font-size: var(--fs-sm);
    line-height: 1;
    color: inherit;
    opacity: var(--chip-fade);
    display: flex;
    align-items: center;
  }
  .chip-x:hover { opacity: 1; }

  .add-wrap {
    position: relative;
  }
  .popup.align-right { left: auto; right: 0; }

  .add-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: none;
    border: 1px solid var(--border-2);
    color: var(--text-2);
    cursor: pointer;
    padding: 0;
    transition: all 0.15s;
  }
  .add-btn:hover { border-color: var(--accent); color: var(--accent); }

  .popup {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    min-width: 240px;
    background: var(--bg-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    box-shadow: var(--shadow-lg);
    z-index: 500;
    padding: var(--sp-2);
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }

  .popup-input {
    width: 100%;
    padding: 0.3rem var(--sp-2);
    font-size: var(--fs-sm);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--text);
    outline: none;
  }
  .popup-input:focus { border-color: var(--accent); }
  .hint { font-size: var(--fs-2xs); color: var(--text-3); padding: 0 0.1rem; }

  .suggestions {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    overflow: hidden;
  }

  .sug-item {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.3rem var(--sp-2);
    background: none;
    border: none;
    cursor: pointer;
    font-size: var(--fs-sm);
    color: var(--text);
    text-align: left;
    transition: background 0.1s;
  }
  .sug-item:hover { background: var(--surface); }

  .sug-folder-label {
    margin-left: auto;
    font-size: var(--fs-2xs);
    color: var(--text-3);
    flex-shrink: 0;
  }

  .sug-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .color-row {
    display: flex;
    gap: 0.3rem;
    flex-wrap: wrap;
  }

  .color-swatch {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 2px solid transparent;
    cursor: pointer;
    padding: 0;
    transition: transform 0.1s;
  }
  .color-swatch.active { border-color: var(--text); transform: scale(1.2); }
  .color-swatch:hover { transform: scale(1.15); }

  .color-swatch-custom {
    position: relative;
    background: conic-gradient(#f43f5e, #f97316, #eab308, #22c55e, #06b6d4, #6366f1, #ec4899, #f43f5e);
    overflow: hidden;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .color-swatch-custom input[type="color"] {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    opacity: 0;
    cursor: pointer;
    border: none;
    padding: 0;
    margin: 0;
  }

  .custom-dot {
    position: absolute;
    inset: 3px;
    border-radius: 50%;
    border: 1.5px solid rgba(255,255,255,0.7);
    pointer-events: none;
  }

  /* .btn.btn-primary covers colors/hover; keep only the left-aligned label delta */
  .create-btn { justify-content: flex-start; text-align: left; }
</style>
