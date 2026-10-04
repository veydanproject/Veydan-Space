<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import ChipMark from '$lib/core/ui/ChipMark.svelte';
  import NoteLabelSheet from '$lib/notes/mobile/NoteLabelSheet.svelte';
  import { api, formatError, type NavChild, type NoteTag } from '$lib/notes/mobile/api';
  import { t } from '$lib/core/mobile/i18n';
  import { parseBinding } from '$lib/core/bindings';
  import { hexColor } from '$lib/core/foreign-labels.svelte';
  import { isSystemTag, mergeTags, systemTags, userLabels } from '$lib/core/entity-tags';

  interface Props {
    tags: string[];
    /** Suggest notes and store `note:id` in tags. Password forms only. */
    linkNotes?: boolean;
    /** Hide the add chip. Existing tags can still be removed. */
    allowAdd?: boolean;
    onchange: (tags: string[]) => void;
  }

  let { tags, linkNotes = false, allowAdd = true, onchange }: Props = $props();

  let open = $state(false);
  let error = $state('');
  let allTags = $state<NoteTag[]>([]);
  let workspaces = $state<NavChild[]>([]);
  let profiles = $state<NavChild[]>([]);
  let notes = $state<{ id: string; title: string }[]>([]);

  const labels = $derived(userLabels(tags));
  const bindings = $derived(systemTags(tags));
  let boundNames = $state<Map<string, string>>(new Map());

  $effect(() => {
    const missing = bindings.filter((tag) => {
      const parsed = parseBinding(tag);
      return parsed && parsed.kind !== 'profile' && parsed.kind !== 'workspace' && !boundNames.has(tag);
    });
    if (missing.length === 0) return;
    let cancelled = false;
    api.notes.bindingSummaries(missing).then((list) => {
      if (cancelled) return;
      const next = new Map(boundNames);
      for (const item of list) next.set(item.binding, item.name);
      for (const tag of missing) if (!next.has(tag)) next.set(tag, tag.slice(tag.indexOf(':') + 1));
      boundNames = next;
    }).catch(() => {});
    return () => { cancelled = true; };
  });

  onMount(() => {
    api.notes.tags().then((list) => (allTags = list)).catch(() => {});
    api.notes.nav().then((nav) => {
      workspaces = nav.all_workspaces;
      profiles = nav.all_profiles;
    }).catch(() => {});
    if (linkNotes) {
      api.notes.list().then((items) => (notes = items.map((n) => ({ id: n.id, title: n.title })))).catch(() => {});
    }
  });

  function bindingKind(tag: string): string {
    return parseBinding(tag)?.kind ?? 'workspace';
  }

  function bindingName(tag: string): string {
    const parsed = parseBinding(tag);
    if (!parsed) return tag;
    if (parsed.kind === 'profile') return profiles.find((item) => item.id === parsed.value)?.name ?? parsed.value;
    if (parsed.kind === 'workspace') return workspaces.find((item) => item.id === parsed.value)?.name ?? parsed.value;
    if (parsed.kind === 'note') return notes.find((item) => item.id === parsed.value)?.title ?? parsed.value;
    return boundNames.get(tag) ?? parsed.value;
  }

  /** A workspace's chip is in its color, as in the rows of the notes; green without one. */
  function bindingColor(tag: string): string | undefined {
    const parsed = parseBinding(tag);
    if (parsed?.kind !== 'workspace') return undefined;
    return hexColor(workspaces.find((item) => item.id === parsed.value)?.color) ?? 'var(--success)';
  }

  function tagColor(name: string): string | undefined {
    return allTags.find((tag) => tag.name === name)?.color;
  }

  function remove(tag: string) {
    onchange(tags.filter((item) => item !== tag));
  }

  async function addTag(name: string, color?: string) {
    const value = name.trim().replace(/^#/, '');
    if (!value || isSystemTag(value) || labels.includes(value)) {
      open = false;
      return;
    }
    error = '';
    const existing = allTags.find((tag) => tag.name === value);
    try {
      if (!existing && color) {
        const created = await api.notes.tagCreate(value, color);
        allTags = [...allTags, created];
      }
      onchange(mergeTags([], bindings, [...labels, existing?.name ?? value]));
      open = false;
    } catch (err) {
      error = formatError(err);
    }
  }

  function addBinding(binding: string) {
    if (!isSystemTag(binding) || bindings.includes(binding)) {
      open = false;
      return;
    }
    onchange(mergeTags([], [...bindings, binding], labels));
    open = false;
  }
</script>

<!-- An .m-field like the fields around it: the caption is their uppercase label -->
<div class="m-field labels">
  <span class="caption">{$t('totp_field_tags')}</span>
  <div class="m-chips">
    {#each bindings as binding (binding)}
      <button
        type="button"
        class="m-chip small tinted"
        style:--chip={bindingColor(binding)}
        onclick={() => remove(binding)}
      >
        <ChipMark kind={bindingKind(binding)} />
        {bindingName(binding)}
      </button>
    {/each}
    {#each labels as name (name)}
      <button type="button" class="m-chip small tinted" style:--chip={tagColor(name)} onclick={() => remove(name)}>
        <ChipMark kind="tag" />
        {name}
      </button>
    {/each}
    {#if allowAdd}
      <button type="button" class="m-chip add" onclick={() => (open = true)} aria-label={$t('notes_tags_add')}>
        <Icon name="plus" size={14} />
      </button>
    {/if}
  </div>
  {#if error}
    <div class="m-error">{error}</div>
  {/if}
</div>

<NoteLabelSheet
  {open}
  tags={allTags}
  selectedTags={labels}
  folders={[]}
  folderIds={[]}
  {workspaces}
  {profiles}
  {bindings}
  notes={linkNotes ? notes : []}
  onclose={() => (open = false)}
  onaddTag={addTag}
  onaddFolder={() => {}}
  onaddBinding={addBinding}
/>

<style>
  .labels { gap: var(--sp-2); }
  /* The look of `.m-field label` (core/styles/mobile.css); a span, as no single control is labelled */
  .caption {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-2);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
</style>
