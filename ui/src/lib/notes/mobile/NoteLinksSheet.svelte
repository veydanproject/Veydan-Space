<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- Outgoing wiki links and backlinks of a note. -->
<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import Notice from '$lib/core/ui/Notice.svelte';
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
    <section class="section">
      <div class="section-label">{label}</div>
      <div class="m-list">
        {#each items as n (n.id)}
          <button type="button" class="list-row dense" onclick={() => onopen(n.id)}>
            <Icon name="notes" size={16} />
            <span class="list-row-label">{n.title || $t('notes_untitled')}</span>
            <Icon name="chevron-right" size={16} />
          </button>
        {/each}
      </div>
    </section>
  {/if}
{/snippet}

<BottomSheet {open} title={$t('notes_links')} {onclose}>
  {#if error}<Notice tone="danger">{error}</Notice>{/if}
  {#if isEmpty}
    <p class="empty-state sm">{$t('notes_links_empty')}</p>
  {:else if links}
    {@render group($t('notes_links_outgoing'), links.outgoing)}
    {@render group($t('notes_links_backlinks'), links.backlinks)}
    {#if links.unresolved.length}
      <section class="section">
        <div class="section-label">{$t('notes_links_unresolved')}</div>
        <div class="m-list">
          {#each links.unresolved as target (target)}
            <button type="button" class="list-row dense" onclick={() => oncreate(target)}>
              <Icon name="plus" size={16} />
              <span class="list-row-label">{target}</span>
              <Icon name="chevron-right" size={16} />
            </button>
          {/each}
        </div>
      </section>
    {/if}
  {/if}
</BottomSheet>

<style>
  .list-row-label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
