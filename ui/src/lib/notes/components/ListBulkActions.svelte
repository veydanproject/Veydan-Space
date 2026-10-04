<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { t, locale, countKey } from '$lib/core/i18n';
  import { ask } from '$lib/core/ui/confirm.svelte';
  import { notesStore } from '$lib/notes/store/notes.svelte';
  import type { NoteListItem } from '$lib/notes/types';

  interface Props {
    /** Current list filter is the trash */
    isTrash: boolean;
    /** Notes shown in the list right now */
    notes: NoteListItem[];
  }

  let { isTrash, notes }: Props = $props();

  let busy = $state(false);

  // The question is the app's own dialog (core/ui/confirm.svelte.ts): it stays
  // inside the window's frame, and the window can be moved while it is open.
  async function run() {
    const n = notes.length;
    const title = isTrash ? $t('notes_btn_empty_trash') : $t('notes_btn_delete_all');
    const message = isTrash
      ? $t(countKey('notes_empty_trash_confirm', n, $locale), { n: String(n) })
      : $t(countKey('notes_delete_all_confirm', n, $locale), { n: String(n) });
    if (!(await ask({ title, message, confirmLabel: title }))) return;
    busy = true;
    try {
      if (isTrash) await notesStore.emptyTrash();
      else await notesStore.deleteMany(notes.map((x) => x.id));
    } finally {
      busy = false;
    }
  }
</script>

{#if isTrash}
  <button class="btn btn-ghost btn-sm danger" disabled={!notes.length || busy} onclick={run}>
    <Icon name="trash" size={14} /> {$t('notes_btn_empty_trash')}
  </button>
{:else}
  <button
    class="icon-btn danger-soft"
    disabled={!notes.length || busy}
    title={$t('notes_btn_delete_all')}
    aria-label={$t('notes_btn_delete_all')}
    onclick={run}
  >
    <Icon name="trash" size={14} />
  </button>
{/if}

<style>
  .btn.danger { color: var(--danger-text); }
  .btn.danger:hover:not(:disabled) { background: var(--danger-bg); }
  .icon-btn { width: 32px; height: 32px; }
</style>
