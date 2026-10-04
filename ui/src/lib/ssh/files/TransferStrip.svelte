<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/i18n';
  import { formatBytes } from '$lib/core/utils';
  import { filesStore } from '$lib/ssh/store/sftp.svelte';

  let transfers = $derived(filesStore.transfers);
  let hasFinished = $derived(transfers.some((t) => t.status !== 'running'));

  function pct(done: number, total: number): number {
    return total > 0 ? Math.min(100, (done / total) * 100) : 0;
  }

  function fileName(path: string): string {
    return path.split('/').pop() || path;
  }
</script>

{#if transfers.length > 0}
  <div class="transfer-strip">
    <div class="strip-header">
      <span class="strip-title">{$t('files_transfers')}</span>
      {#if hasFinished}
        <button class="btn btn-ghost btn-sm" onclick={() => filesStore.clearFinishedTransfers()}>
          {$t('files_transfers_clear')}
        </button>
      {/if}
    </div>
    {#each transfers as tr (tr.id)}
      <div class="transfer-row">
        <span class="tr-icon" class:error={tr.status === 'error'}>
          <Icon name={tr.kind === 'download' ? 'download' : 'upload'} size={14} />
        </span>
        <div class="tr-main">
          <div class="tr-line">
            <span class="tr-label" title={tr.currentFile}>
              {tr.label}
              {#if tr.filesTotal > 1}
                <span class="tr-files">{tr.filesDone}/{tr.filesTotal}</span>
              {/if}
              {#if tr.status === 'running' && tr.currentFile}
                <span class="tr-current">· {fileName(tr.currentFile)}</span>
              {/if}
            </span>
            <span class="tr-stats">
              {#if tr.status === 'running'}
                {formatBytes(tr.bytesDone)} / {formatBytes(tr.bytesTotal)}
                {#if tr.speed > 0}· {formatBytes(tr.speed)}/s{/if}
              {:else if tr.status === 'done'}
                <span class="status-ok">
                  {$t('files_transfer_done')}{tr.filesSkipped > 0 ? ` · ${$t('files_transfer_skipped', { n: String(tr.filesSkipped) })}` : ''}
                </span>
              {:else if tr.status === 'cancelled'}
                <span class="status-warn">{$t('files_transfer_cancelled')}</span>
              {:else}
                <span class="status-err" title={tr.error}>{tr.error}</span>
              {/if}
            </span>
          </div>
          <div class="tr-bar">
            <div
              class="tr-fill"
              class:done={tr.status === 'done'}
              class:failed={tr.status === 'error'}
              class:paused={tr.status === 'cancelled'}
              style="width:{pct(tr.bytesDone, tr.bytesTotal)}%"
            ></div>
          </div>
        </div>
        {#if tr.status === 'running'}
          <button class="icon-btn danger" title={$t('cancel')} onclick={() => filesStore.cancelTransfer(tr.id)}>
            <Icon name="x" size={13} />
          </button>
        {/if}
      </div>
    {/each}
  </div>
{/if}

<style>
  .transfer-strip {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    padding: var(--sp-2) var(--sp-3);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    max-height: 180px;
    overflow-y: auto;
  }

  .strip-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .strip-title {
    font-size: var(--fs-xs);
    font-weight: var(--fw-semibold);
    text-transform: uppercase;
    letter-spacing: 0.4px;
    color: var(--text-2);
  }

  .transfer-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .tr-icon {
    flex-shrink: 0;
    color: var(--accent-text);
    display: inline-flex;
  }
  .tr-icon.error { color: var(--danger-text); }

  .tr-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .tr-line {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--sp-3);
    font-size: var(--fs-sm);
    min-width: 0;
  }
  .tr-label {
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .tr-files { color: var(--text-2); }
  .tr-current { color: var(--text-3); font-size: var(--fs-xs); }
  .tr-stats {
    flex-shrink: 0;
    color: var(--text-2);
    font-size: var(--fs-xs);
    font-family: var(--font-mono);
  }
  .status-ok { color: var(--success-text); }
  .status-warn { color: var(--warn-text); }
  .status-err { color: var(--danger-text); }

  .tr-bar {
    height: 4px;
    background: var(--surface-3);
    border-radius: 2px;
    overflow: hidden;
  }
  .tr-fill {
    height: 100%;
    background: var(--accent);
    border-radius: 2px;
    transition: width 0.15s linear;
  }
  .tr-fill.done { background: var(--success); }
  .tr-fill.failed { background: var(--danger); }
  .tr-fill.paused { background: var(--warn-text); }
</style>
