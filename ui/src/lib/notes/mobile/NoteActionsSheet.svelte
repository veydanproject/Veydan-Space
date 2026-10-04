<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/mobile/i18n';
  import BottomSheet from '$lib/core/mobile/BottomSheet.svelte';

  interface Props {
    open: boolean;
    noteId: string;
    pinned: boolean;
    archived: boolean;
    onclose: () => void;
    onhistory: () => void;
    onpin: () => void;
    onarchive: () => void;
    onmove: () => void;
    /** Left out where there is no workspace to add the note to (Notes alone has none). */
    onworkspace?: () => void;
    ondelete: () => void;
  }

  let { open, noteId, pinned, archived, onclose, onhistory, onpin, onarchive, onmove, onworkspace, ondelete }: Props = $props();

  const isNew = $derived(noteId === 'new');
</script>

<BottomSheet {open} title={$t('notes_actions')} {onclose}>
  <div class="m-list">
    <button type="button" class="m-row" disabled={isNew} onclick={onpin}>
      <Icon name="pin" size={20} /><span class="m-row-label">{pinned ? $t('notes_unpin') : $t('notes_pin')}</span>
    </button>
    <button type="button" class="m-row" disabled={isNew} onclick={onmove}>
      <Icon name="folder" size={20} /><span class="m-row-label">{$t('notes_move')}</span>
    </button>
    {#if onworkspace}
      <button type="button" class="m-row" onclick={onworkspace}>
        <Icon name="layers" size={20} /><span class="m-row-label">{$t('notes_workspace_add')}</span>
      </button>
    {/if}
    <button type="button" class="m-row" disabled={isNew} onclick={onarchive}>
      <Icon name={archived ? 'archive-restore' : 'archive'} size={20} /><span class="m-row-label">{archived ? $t('notes_unarchive') : $t('notes_archive')}</span>
    </button>
    <button type="button" class="m-row" disabled={isNew} onclick={onhistory}>
      <Icon name="clock" size={20} /><span class="m-row-label">{$t('notes_history')}</span>
    </button>
  </div>

  <div class="m-list">
    <button type="button" class="m-row" onclick={ondelete}>
      <Icon name="trash-2" size={20} /><span class="m-row-label danger">{$t('notes_delete')}</span>
    </button>
  </div>
</BottomSheet>

<style>
  .m-row:disabled { opacity: 0.45; }
  .danger { color: var(--danger-text); }
</style>
