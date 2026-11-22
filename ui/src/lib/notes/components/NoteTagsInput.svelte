<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import type { NoteTag, NoteTagInfo, NoteFolder } from '$lib/notes/types';
  import { PALETTE } from '$lib/core/palette';
  import { api } from '$lib/notes/api';
  import Icon from '$lib/core/Icon.svelte';
  import { t, type TranslationKey } from '$lib/core/i18n';
  import ChipMark from '$lib/core/ui/ChipMark.svelte';
  import Suggest, { type SuggestItem } from '$lib/core/ui/Suggest.svelte';
  import ColorSwatches from '$lib/core/ui/ColorSwatches.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
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

  let open = $state(false);
  let inputValue = $state('');
  let selectedColor = $state(PALETTE[0]);
  let inputEl: HTMLInputElement | null = $state(null);
  /**
   * The list comes with the field's focus, after the dialog: one mounted with it would reach the body
   * before the dialog freezes the body (layer.ts) and be frozen too, its rows dead to the mouse.
   */
  let listed = $state(false);

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

  function openPopup() {
    open = true;
    listed = false;
    inputValue = '';
    selectedColor = PALETTE[0];
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
</script>

<div class="tags-outer">
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

  <button class="icon-btn" onclick={openPopup} title={$t('notes_label_title')}>
    <Icon name="plus" size={18} />
  </button>
</div>

<Dialog bind:open title={$t('notes_label_title')} onclose={close}>
  <div class="popup">
    <input
      bind:this={inputEl}
      bind:value={inputValue}
      type="text"
      placeholder={pickerText.placeholder}
      onkeydown={onKeydown}
      onfocus={() => (listed = true)}
    />
    {#if !inputValue.trim() && entitySuggestions.length === 0}
      <div class="hint">{pickerText.hint}</div>
    {/if}

    {#if listed && items.length > 0}
      <Suggest bind:this={list} {items} anchor={inputEl} label={$t('notes_label_title')} onpick={pick} onclose={close}>
        {#snippet extra()}
          <div class="color-row"><ColorSwatches bind:value={selectedColor} label={$t('notes_label_title')} /></div>
        {/snippet}
      </Suggest>
    {/if}
  </div>
</Dialog>

<style>
  /* The chips have a row of their own under the title (NoteEditor): they wrap, nothing is cut */
  .tags-outer {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sp-1);
    flex: 1;
    min-width: 0;
  }

  .tags-wrap { display: contents; }

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

  .popup {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1-5);
  }

  .hint { padding: 0 var(--sp-0-5); }

  .color-row { padding: var(--sp-1) var(--sp-2); }

</style>
