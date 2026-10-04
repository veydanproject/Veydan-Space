<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Outgoing wiki links and backlinks of a note. -->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { api, formatError, type NoteLinks, type NoteListItem } from '$lib/notes/mobile/api';
  import { t } from '$lib/core/mobile/i18n';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';

  interface Props {
    open: boolean;
    noteId: string;
    onclose: () => void;
    onopen: (id: string) => void;
    /** Create the missing note for an unresolved `[[target]]` and open it */
    oncreate: (target: string) => void;
  }

  let { open, noteId, onclose, onopen, oncreate }: Props = $props();

  let links = $state<NoteLinks | null>(null);
  let error = $state('');

  $effect(() => {
    if (!open) return;
    api.notes.links(noteId).then((l) => (links = l)).catch((e) => (error = formatError(e)));
  });

  const isEmpty = $derived(
    !!links && links.outgoing.length === 0 && links.backlinks.length === 0 && links.unresolved.length === 0,
  );
</script>

{#snippet group(label: string, items: NoteListItem[])}
  {#if items.length}
    <div>
      <div class="group">{label}</div>
      <div class="m-list">
        {#each items as n (n.id)}
          <button type="button" class="m-row" onclick={() => onopen(n.id)}>
            <Icon name="file-text" size={16} />
            <span class="m-row-label">{n.title || $t('notes_untitled')}</span>
            <Icon name="chevron-right" size={16} />
          </button>
        {/each}
      </div>
    </div>
  {/if}
{/snippet}

<BottomSheet {open} title={$t('notes_links')} {onclose}>
  {#if error}<div class="m-error">{error}</div>{/if}
  {#if isEmpty}
    <p class="empty">{$t('notes_links_empty')}</p>
  {:else if links}
    {@render group($t('notes_links_outgoing'), links.outgoing)}
    {@render group($t('notes_links_backlinks'), links.backlinks)}
    {#if links.unresolved.length}
      <div>
        <div class="group">{$t('notes_links_unresolved')}</div>
        <div class="m-list">
          {#each links.unresolved as target (target)}
            <button type="button" class="m-row" onclick={() => oncreate(target)}>
              <Icon name="plus" size={16} />
              <span class="m-row-label">{target}</span>
              <Icon name="chevron-right" size={16} />
            </button>
          {/each}
        </div>
      </div>
    {/if}
  {/if}
</BottomSheet>

<style>
  .empty { color: var(--text-3); text-align: center; padding: var(--sp-5) 0; }
  .group {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-2);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin-bottom: var(--sp-1);
  }
  .m-row-label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
