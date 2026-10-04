<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Labels and references of a password or TOTP entry. The editor comes from
  the module that keeps the labels (the notes, through the registry). A
  product without one (Pass) edits the free labels here: chips with a remove
  button and a field to type a new one; a reference shows as 10.3 says.
-->
<script lang="ts">
  import type { LabelFieldProps } from '$lib/core/module';
  import { registry } from '$lib/core/registry';
  import Icon from '$lib/core/Icon.svelte';
  import ChipMark from '$lib/core/ui/ChipMark.svelte';
  import { t } from '$lib/core/mobile/i18n';
  import { directory } from '$lib/core/directory';
  import { parseBinding } from '$lib/core/bindings';
  import { isSystemTag, mergeTags, systemTags, userLabels } from '$lib/core/entity-tags';

  let { tags, linkNotes = false, allowAdd = true, onchange }: LabelFieldProps = $props();

  const Field = registry.labelField();

  let draft = $state('');
  const labels = $derived(userLabels(tags));
  const bindings = $derived(systemTags(tags));

  function add() {
    const names = draft.split(',').map((part) => part.trim()).filter((part) => part && !isSystemTag(part));
    draft = '';
    if (names.length) onchange(mergeTags([], bindings, [...labels, ...names]));
  }

  function remove(tag: string) {
    onchange(tags.filter((item) => item !== tag));
  }

  /** The name of the kind a reference points at ("Profile"), read out before the chip's text. */
  function bindingCaption(tag: string): string | undefined {
    const parsed = parseBinding(tag);
    const kind = parsed ? directory.kind(parsed.kind) : undefined;
    return kind ? $t(kind.label) : undefined;
  }

  /** A workspace's chip is in its color (the owner's, or its label's without Space, 10.3); neutral without one. */
  function bindingColor(tag: string): string | undefined {
    const parsed = parseBinding(tag);
    return parsed?.kind === 'workspace' ? directory.color('workspace', parsed.value) : undefined;
  }

  function bindingName(tag: string): string {
    const parsed = parseBinding(tag);
    return parsed ? directory.name(parsed.kind, parsed.value, $t) : tag;
  }
</script>

{#if Field}
  <Field {tags} {linkNotes} {allowAdd} {onchange} />
{:else if tags.length || allowAdd}
  <div class="m-field">
    <label for="labels-new">{$t('pw_field_tags')}</label>
    {#if tags.length}
      <div class="m-chips">
        {#each bindings as tag (tag)}
          {@const color = bindingColor(tag)}
          <span class="m-chip small tinted" class:neutral={!color} style:--chip={color}>
            <ChipMark kind={parseBinding(tag)?.kind ?? 'tag'} caption={bindingCaption(tag)} />
            {bindingName(tag)}
            <button type="button" class="x" onclick={() => remove(tag)} aria-label={$t('pass_remove')}><Icon name="x" size={14} /></button>
          </span>
        {/each}
        {#each labels as label (label)}
          {@const color = directory.summary('note_tag', label)?.color}
          <span class="m-chip small tinted" style:--chip={color}>
            {label}
            <button type="button" class="x" onclick={() => remove(label)} aria-label={$t('pass_remove')}><Icon name="x" size={14} /></button>
          </span>
        {/each}
      </div>
    {/if}
    {#if allowAdd}
      <div class="add">
        <input
          id="labels-new"
          bind:value={draft}
          placeholder={$t('notes_tags_placeholder')}
          autocomplete="off"
          autocapitalize="off"
          enterkeyhint="done"
          onkeydown={(e) => { if (e.key === 'Enter' || e.key === ',') { e.preventDefault(); add(); } }}
          onblur={add}
        />
      </div>
    {/if}
  </div>
{/if}

<style>
  .m-chips { margin-bottom: var(--sp-1); }
  .m-chip.small { position: relative; min-height: 32px; padding-right: 0; font-size: 12px; }
  .x {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    opacity: var(--chip-fade);
  }
  .add input { width: 100%; }
</style>
