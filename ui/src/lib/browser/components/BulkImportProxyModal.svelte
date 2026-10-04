<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { proxiesStore } from '$lib/browser/store/proxies.svelte';
  import { parseProxyList, toBulkItems, type ProxyImportType } from '$lib/browser/proxyImport';
  import { formatError } from '$lib/core/utils';

  interface Props {
    open: boolean;
    onclose: () => void;
  }

  let { open, onclose }: Props = $props();

  const TYPES: ProxyImportType[] = ['http', 'https', 'socks5'];
  const MAX_RENDERED_ROWS = 200;
  const CHECK_CONCURRENCY = 4;

  type CheckState = 'pending' | 'ok' | 'failed';
  interface ResultRow {
    lineNumber: number;
    raw: string;
    status: 'imported' | 'duplicate' | 'error';
    message?: string;
    id?: string;
    check?: CheckState;
  }

  let text = $state('');
  let defaultType = $state<ProxyImportType>('socks5');
  let checkAfter = $state(false);
  let phase = $state<'edit' | 'importing' | 'done'>('edit');
  let error = $state('');
  let fileInput = $state<HTMLInputElement | null>(null);

  let resultRows = $state<ResultRow[]>([]);
  let checking = $state(false);
  let checkDone = $state(0);
  let checkTotal = $state(0);
  let checkOk = $state(0);
  let checkFailed = $state(0);

  let parsed = $derived(parseProxyList(text, defaultType));
  let validCount = $derived(parsed.filter((l) => l.ok).length);
  let invalidLines = $derived(parsed.filter((l) => !l.ok));

  let importedCount = $derived(resultRows.filter((r) => r.status === 'imported').length);
  let duplicateCount = $derived(resultRows.filter((r) => r.status === 'duplicate').length);
  let errorCount = $derived(resultRows.filter((r) => r.status === 'error').length);

  function reset() {
    text = '';
    defaultType = 'socks5';
    checkAfter = false;
    phase = 'edit';
    error = '';
    resultRows = [];
    checking = false;
    checkDone = checkTotal = checkOk = checkFailed = 0;
  }

  $effect(() => {
    if (!open) reset();
  });

  async function handleFileChange(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    const content = await file.text();
    text = text.trim() ? `${text.replace(/\n$/, '')}\n${content}` : content;
  }

  async function doImport() {
    phase = 'importing';
    error = '';
    try {
      const result = await proxiesStore.bulkImport(toBulkItems(parsed));

      const rawByLine = new Map(parsed.map((l) => [l.lineNumber, l.raw]));
      const rows: ResultRow[] = result.rows.map((r) => ({
        lineNumber: r.line_number,
        raw: rawByLine.get(r.line_number) ?? '',
        status: r.status,
        message: r.message,
        id: r.id,
      }));
      for (const l of parsed) {
        if (!l.ok) rows.push({ lineNumber: l.lineNumber, raw: l.raw, status: 'error', message: $t(l.errorKey) });
      }
      rows.sort((a, b) => a.lineNumber - b.lineNumber);
      resultRows = rows;
      phase = 'done';

      if (checkAfter && result.imported.length > 0) {
        await runChecks(result.imported.map((p) => p.id));
      }
    } catch (e) {
      error = formatError(e);
      phase = 'edit';
    }
  }

  function setCheck(id: string, check: CheckState) {
    resultRows = resultRows.map((r) => (r.id === id ? { ...r, check } : r));
  }

  async function runChecks(ids: string[]) {
    checking = true;
    checkTotal = ids.length;
    checkDone = checkOk = checkFailed = 0;
    resultRows = resultRows.map((r) => (r.id && ids.includes(r.id) ? { ...r, check: 'pending' } : r));

    const queue = [...ids];
    const worker = async () => {
      for (let id = queue.shift(); id !== undefined; id = queue.shift()) {
        try {
          await proxiesStore.check(id);
          setCheck(id, 'ok');
          checkOk++;
        } catch {
          proxiesStore.markFailed(id);
          setCheck(id, 'failed');
          checkFailed++;
        }
        checkDone++;
      }
    };
    await Promise.all(Array.from({ length: Math.min(CHECK_CONCURRENCY, ids.length) }, worker));
    checking = false;
  }
</script>

<Dialog {open} title={$t('proxy_import_title')} width="520px" {onclose} closeOnBackdrop={phase === 'edit'}>

  {#if phase === 'edit' || phase === 'importing'}
    <div class="import-form">
      <textarea
        class="proxy-input mono"
        placeholder={$t('proxy_import_placeholder')}
        bind:value={text}
        rows="9"
        spellcheck="false"
        disabled={phase === 'importing'}
      ></textarea>

      <div class="form-row">
        <button class="btn btn-ghost btn-sm" onclick={() => fileInput?.click()} disabled={phase === 'importing'}>
          <Icon name="upload" size={13} />
          {$t('proxy_import_load_file')}
        </button>
        <input
          type="file"
          accept=".txt,.csv,.list,text/plain"
          style="display:none"
          bind:this={fileInput}
          onchange={handleFileChange}
        />

        <div class="type-select" title={$t('proxy_import_default_type_hint')}>
          <span class="type-select-label">{$t('proxy_import_default_type')}</span>
          <div class="seg">
            {#each TYPES as tp (tp)}
              <button
                class="seg-btn"
                class:active={defaultType === tp}
                onclick={() => (defaultType = tp)}
                disabled={phase === 'importing'}
              >{tp}</button>
            {/each}
          </div>
        </div>
      </div>

      {#if parsed.length > 0}
        <div class="preview-summary">
          {$t('proxy_import_preview_summary', {
            total: String(parsed.length),
            valid: String(validCount),
            invalid: String(invalidLines.length),
          })}
        </div>
        {#if invalidLines.length > 0}
          <div class="line-list">
            {#each invalidLines.slice(0, MAX_RENDERED_ROWS) as line (line.lineNumber)}
              <div class="line-row">
                <span class="line-num">{line.lineNumber}</span>
                <code class="line-raw">{line.raw}</code>
                <span class="status-badge status-failed">{$t(line.errorKey)}</span>
              </div>
            {/each}
            {#if invalidLines.length > MAX_RENDERED_ROWS}
              <div class="more-lines">{$t('proxy_import_more_lines', { n: String(invalidLines.length - MAX_RENDERED_ROWS) })}</div>
            {/if}
          </div>
        {/if}
      {/if}

      <div class="check-after">
        <span>{$t('proxy_import_check_after')}</span>
        <button
          class="toggle"
          class:on={checkAfter}
          onclick={() => (checkAfter = !checkAfter)}
          aria-pressed={checkAfter}
          aria-label={$t('proxy_import_check_after')}
          disabled={phase === 'importing'}
        ></button>
      </div>

      {#if error}
        <div class="error-msg">{error}</div>
      {/if}
    </div>
  {:else}
    <div class="import-result">
      <div class="result-summary">
        {$t('proxy_import_result_summary', {
          total: String(resultRows.length),
          imported: String(importedCount),
          duplicates: String(duplicateCount),
          errors: String(errorCount),
        })}
      </div>

      {#if checking}
        <div class="check-progress">
          <Icon name="refresh-cw" size={13} />
          {$t('proxy_import_checking_progress', { done: String(checkDone), total: String(checkTotal) })}
        </div>
      {:else if checkTotal > 0}
        <div class="check-progress">
          {$t('proxy_import_check_summary', {
            ok: String(checkOk),
            total: String(checkTotal),
            failed: String(checkFailed),
          })}
        </div>
      {/if}

      <div class="line-list">
        {#each resultRows.slice(0, MAX_RENDERED_ROWS) as row (row.lineNumber)}
          <div class="line-row">
            <span class="line-num">{row.lineNumber}</span>
            <code class="line-raw">{row.raw}</code>
            {#if row.status === 'imported'}
              {#if row.check === 'ok'}
                <span class="status-badge status-active">{$t('proxy_import_check_ok')}</span>
              {:else if row.check === 'failed'}
                <span class="status-badge status-failed">{$t('proxy_import_check_failed')}</span>
              {:else if row.check === 'pending'}
                <span class="status-badge status-unknown">{$t('proxy_import_check_pending')}</span>
              {:else}
                <span class="status-badge status-active">{$t('proxy_import_status_imported')}</span>
              {/if}
            {:else if row.status === 'duplicate'}
              <span class="status-badge status-unknown">{$t('proxy_import_status_duplicate')}</span>
            {:else}
              <span class="status-badge status-failed" title={row.message}>{row.message ?? $t('proxy_import_status_error')}</span>
            {/if}
          </div>
        {/each}
        {#if resultRows.length > MAX_RENDERED_ROWS}
          <div class="more-lines">{$t('proxy_import_more_lines', { n: String(resultRows.length - MAX_RENDERED_ROWS) })}</div>
        {/if}
      </div>
    </div>
  {/if}

  {#snippet footer()}
    {#if phase === 'done'}
      <button class="btn btn-primary" onclick={onclose}>{$t('pwgen_btn_close')}</button>
    {:else}
      <button class="btn btn-ghost" onclick={onclose} disabled={phase === 'importing'}>{$t('proxy_btn_cancel')}</button>
      <button class="btn btn-primary" disabled={validCount === 0 || phase === 'importing'} onclick={doImport}>
        {phase === 'importing'
          ? $t('proxy_import_btn_loading')
          : $t('proxy_import_btn', { n: String(validCount) })}
      </button>
    {/if}
  {/snippet}

</Dialog>

<style>
  .import-form, .import-result {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }

  .proxy-input {
    width: 100%;
    background: var(--surface-3);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text);
    font-size: var(--fs-sm);
    padding: 0.6rem var(--sp-3);
    resize: vertical;
    box-sizing: border-box;
  }
  .proxy-input:focus {
    border-color: var(--accent-border);
    box-shadow: 0 0 0 3px var(--accent-bg);
  }

  .form-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    flex-wrap: wrap;
  }

  .type-select {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }
  .type-select-label {
    font-size: var(--fs-sm);
    color: var(--text-3);
  }
  .type-select .seg-btn { font-family: var(--font-mono); }

  .preview-summary, .result-summary {
    font-size: var(--fs-sm);
    color: var(--text-2);
  }
  .result-summary { font-weight: var(--fw-semibold); color: var(--text); }

  .check-progress {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: var(--fs-sm);
    color: var(--text-2);
  }

  .check-after {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
    font-size: var(--fs-sm);
    color: var(--text-body);
  }

  .line-list {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--surface-2);
    max-height: 260px;
    overflow-y: auto;
  }
  .line-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: 0.35rem var(--sp-3);
    border-bottom: 1px solid var(--border);
    min-width: 0;
  }
  .line-row:last-child { border-bottom: none; }

  .line-num {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-3);
    width: 26px;
    flex-shrink: 0;
    text-align: right;
  }
  .line-raw {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-body);
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .status-badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 0.68rem;
    font-weight: var(--fw-bold);
    text-transform: uppercase;
    letter-spacing: 0.4px;
    padding: 3px 9px;
    border-radius: var(--radius-sm);
    flex-shrink: 0;
    max-width: 45%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .status-badge::before { content: ''; width: 5px; height: 5px; border-radius: 50%; background: currentColor; flex-shrink: 0; }
  .status-active { background: var(--success-bg); color: var(--success-text); }
  .status-failed { background: var(--danger-bg); color: var(--danger-text); }
  .status-unknown { background: var(--surface-3); color: var(--text-2); }

  .more-lines {
    padding: 0.35rem var(--sp-3);
    font-size: var(--fs-xs);
    color: var(--text-3);
  }
</style>
