<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { t } from '$lib/core/i18n';
  import { filesStore } from '$lib/ssh/store/sftp.svelte';

  let applyAll = $state(false);

  let conflict = $derived(filesStore.conflict);

  function decide(decision: 'overwrite' | 'skip' | 'abort') {
    filesStore.resolveConflict(decision, decision === 'abort' ? false : applyAll);
    applyAll = false;
  }
</script>

<Dialog
  open={conflict !== null}
  title={$t('files_conflict_title')}
  onclose={() => decide('abort')}
>
  {#if conflict}
    <p class="conflict-msg">
      {$t(conflict.isDir ? 'files_conflict_msg_dir' : 'files_conflict_msg_file', { name: conflict.name })}
    </p>
    {#if conflict.remaining > 0}
      <label class="apply-all">
        <input type="checkbox" bind:checked={applyAll} />
        {$t('files_conflict_apply_all', { n: String(conflict.remaining) })}
      </label>
    {/if}
  {/if}
  {#snippet footer()}
    <button class="btn btn-ghost" onclick={() => decide('abort')}>{$t('cancel')}</button>
    <button class="btn btn-ghost" onclick={() => decide('skip')}>{$t('files_conflict_skip')}</button>
    <button class="btn btn-danger" onclick={() => decide('overwrite')}>{$t('files_conflict_overwrite')}</button>
  {/snippet}
</Dialog>

<style>
  .conflict-msg {
    color: var(--text);
    font-size: var(--fs-base);
    margin-bottom: var(--sp-3);
    word-break: break-word;
  }
  .apply-all {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    color: var(--text-2);
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .apply-all input[type='checkbox'] {
    flex-shrink: 0;
  }
</style>
