<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import type { NoteTag, NoteTagInfo, NoteFolder } from '$lib/notes/types';
  import { api } from '$lib/notes/api';
  import Icon from '$lib/core/Icon.svelte';
  import { t, type TranslationKey } from '$lib/core/i18n';
  import ChipMark from '$lib/core/ui/ChipMark.svelte';
  import Suggest, { type SuggestItem } from '$lib/core/ui/Suggest.svelte';
  import { binding, type EntityKind } from '$lib/core/bindings';
  import { entityKind, entityKinds, ensureEntitiesLoaded, searchEntities, type EntitySummary } from '$lib/notes/context';
  import { labelPickerText } from '$lib/notes/label-kinds';
  import { findTag, normalizeTagName, tagChoice } from '$lib/notes/tag-name';

  interface ContextChip {
    kind: string;
    label: string;
    /** The user's colour (a folder, a workspace); none for an entity or a domain: neutral, told by its mark. */
    color?: string;
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
    // The kit's presets (N.colors); the colour is the user's, never the kind.
    '#8b7bff', '#5ab0f0', '#59c28a', '#e8a33d', '#e66b6b', '#d58ad6',
  ];

  let open = $state(false);
  let inputValue = $state('');
  let selectedColor = $state(TAG_COLORS[0]);
  let inputEl: HTMLInputElement | null = $state(null);
  let wrapEl: HTMLDivElement | null = $state(null);

  /** Lowercase, as stored; a tag named in an old file may still have capitals. */
  const selectedNames = $derived(new Set(selectedTags.map((t) => normalizeTagName(t.name))));

  const suggestions = $derived(
    allTags
      .filter((t) => !selectedNames.has(normalizeTagName(t.name)) && t.name.toLowerCase().includes(inputValue.toLowerCase()))
  );

  const folderSuggestions = $derived(
    inputValue.trim().length > 0
      ? folders
          .filter(f => !activeFolderIds.includes(f.id) && f.name.toLowerCase().includes(inputValue.toLowerCase()))
      : []
  );

  /** Veydan entities matching the input. With an empty field, offer TOTP and passwords to bind. */
  const entitySuggestions = $derived.by(() => {
    const q = inputValue.trim();
    const kinds = (q ? entityKinds().map((d) => d.kind) : ['totp', 'password']) as EntityKind[];
    const out: { binding: string; kind: EntityKind; entity: EntitySummary }[] = [];
    for (const kind of kinds) {
      for (const entity of searchEntities(kind, q, Infinity)) {
        const b = binding(kind, entity.id);
        if (entityKind(kind) && !activeBindings.includes(b)) out.push({ binding: b, kind, entity });
      }
    }
    return out;
  });

  /** Placeholder and hint name only the kinds this product has entities of. */
  const pickerText = $derived(labelPickerText(entityKinds().filter((d) => d.list().length > 0).map((d) => d.kind), $t));

  const isNew = $derived(
    inputValue.trim().length > 0 && !findTag(allTags, inputValue)
  );

  let list = $state<Suggest | null>(null);
  const items = $derived<SuggestItem[]>([
    ...suggestions.map((s) => ({ key: `tag:${s.id}`, text: s.name, dot: s.color, group: $t('suggest_group_note_tag') })),
    ...folderSuggestions.map((f) => ({ key: `folder:${f.id}`, text: f.name, dot: f.color, group: $t('suggest_group_note_folder') })),
    ...entitySuggestions.map((s) => ({
      key: s.binding, text: s.entity.name, group: $t(`suggest_group_${s.kind}` as TranslationKey),
      ...(s.entity.color ? { dot: s.entity.color } : { mark: s.kind }),
    })),
    ...(isNew ? [{ key: 'create', text: $t('notes_tag_add'), create: true }] : []),
  ]);

  /** A row of the list: a tag, a folder, an entity to bind, or the new tag of what is typed. */
  function pick(item: SuggestItem) {
    if (item.create) void addTag(inputValue, selectedColor);
    else if (item.key.startsWith('tag:')) void addTag(item.text);
    else if (item.key.startsWith('folder:')) { onaddFolder?.(item.key.slice('folder:'.length)); close(); }
    else { onaddBinding?.(item.key); close(); }
  }

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
    // "Работа" is the tag "работа": the new tag and the note get one name.
    const choice = tagChoice(name, allTags, [...selectedNames]);
    if (!choice) { close(); return; }

    if (!choice.existing && color) {
      await api.notes.tagCreate(choice.name, color);
    }

    onchange([...selectedNames, choice.name]);
    close();
  }

  function removeTag(name: string) {
    const key = normalizeTagName(name);
    onchange([...selectedNames].filter((n) => n !== key));
  }

  function onKeydown(e: KeyboardEvent) {
    if (list?.keydown(e)) return;
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

        {#if items.length > 0}
          <Suggest bind:this={list} {items} anchor={inputEl} label={$t('notes_label_title')} onpick={pick} onclose={close}>
            {#snippet extra()}
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
            {/snippet}
          </Suggest>
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
    font-size: var(--fs-xs);
    line-height: var(--lh-icon);
    padding: 0.2rem 0.45rem;
    border-radius: var(--radius-pill);
    border-width: 1px;
    border-style: solid;
    font-weight: var(--fw-medium);
    white-space: nowrap;
  }

  .chip-x {
    background: none;
    border: none;
    cursor: pointer;
    padding: 0;
    font-size: var(--fs-sm);
    line-height: var(--lh-icon);
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
  .add-btn:hover { border-color: var(--accent); color: var(--accent-text); }

  .popup {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    min-width: 240px;
    background: var(--bg-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
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
    border-radius: var(--radius-field);
    color: var(--text);
    outline: none;
  }
  .popup-input:focus { border-color: var(--accent-border); }
  .hint { padding: 0 0.1rem; }

  .color-row {
    display: flex;
    gap: 0.3rem;
    flex-wrap: wrap;
    padding: var(--sp-1) var(--sp-2);
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
</style>
