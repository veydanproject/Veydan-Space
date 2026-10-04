<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { api, type SyncDebugEntry } from '$lib/core/api';
  import { t } from '$lib/core/i18n';
  import { isMobile } from '$lib/core/platform';
  import { formatError } from '$lib/core/utils';

  const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

  let enabled = $state(false);
  let entries = $state<SyncDebugEntry[]>([]);
  let error = $state('');
  let copied = $state(false);
  let busy = $state(false);
  let logEl = $state<HTMLDivElement | null>(null);
  let stick = true;
  let copyTimer: ReturnType<typeof setTimeout>;

  function append(row: SyncDebugEntry) {
    if (entries.some((item) => item.seq === row.seq)) return;
    entries = [...entries, row].slice(-400);
  }

  function stamp(at: string): string {
    const d = new Date(at);
    if (Number.isNaN(d.getTime())) return at;
    const p = (n: number, w = 2) => String(n).padStart(w, '0');
    return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${p(d.getMilliseconds(), 3)}`;
  }

  function lineText(row: SyncDebugEntry): string {
    const cycle = row.cycle ? String(row.cycle) : '—';
    return `${stamp(row.at)}  ${cycle}  ${row.source}  ${row.step}  ${row.level}  ${row.message}`;
  }

  function onScroll(e: Event) {
    const el = e.currentTarget as HTMLDivElement;
    stick = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
  }

  $effect(() => {
    entries.length;
    if (stick && logEl) logEl.scrollTop = logEl.scrollHeight;
  });

  onMount(() => {
    if (!isTauri) return;
    let stop = false;
    let unlisten: (() => void) | null = null;
    const pending: SyncDebugEntry[] = [];
    let ready = false;
    void (async () => {
      const { listen } = await import('@tauri-apps/api/event');
      unlisten = await listen<SyncDebugEntry>('sync://debug', (e) => {
        if (!ready) pending.push(e.payload);
        else append(e.payload);
      });
      if (stop) {
        unlisten();
        return;
      }
      try {
        const snap = await api.sync.debugGet();
        enabled = snap.enabled;
        entries = snap.entries;
      } catch (e) {
        error = formatError(e);
      }
      ready = true;
      for (const row of pending) append(row);
    })();
    return () => {
      stop = true;
      unlisten?.();
      clearTimeout(copyTimer);
    };
  });

  async function toggle() {
    if (busy) return;
    busy = true;
    error = '';
    try {
      await api.sync.debugSet(!enabled);
      enabled = !enabled;
    } catch (e) {
      error = formatError(e);
    } finally {
      busy = false;
    }
  }

  async function clearLog() {
    error = '';
    try {
      await api.sync.debugClear();
      entries = (await api.sync.debugGet()).entries;
    } catch (e) {
      error = formatError(e);
    }
  }

  async function copyLog() {
    try {
      await navigator.clipboard.writeText(entries.map(lineText).join('\n'));
      copied = true;
      clearTimeout(copyTimer);
      copyTimer = setTimeout(() => (copied = false), 1500);
    } catch (e) {
      error = formatError(e);
    }
  }
</script>

<div class={isMobile ? 'm-page' : 'page'} style:--page-max={isMobile ? undefined : 'var(--page-max-narrow)'}>
  {#if isMobile}
    <div class="m-header">
      <a class="m-ibtn" href="/settings" aria-label={$t('dev_sync_back')}>
        <Icon name="chevron-left" size={24} />
      </a>
      <h1 class="m-title">{$t('dev_sync_title')}</h1>
    </div>
  {/if}

  <div class={isMobile ? 'm-body body' : 'body'}>
    {#if !isMobile}
      <div class="page-header">
        <a class="back" href="/settings">{$t('dev_sync_back')}</a>
        <h1>{$t('dev_sync_title')}</h1>
      </div>
    {/if}

    <div class="card panel">
      <div class="toggle-row">
        <div class="toggle-info">
          <span>{$t('dev_sync_enable')}</span>
          <span class="hint">{$t('dev_sync_enable_hint')}</span>
        </div>
        <button
          type="button"
          class="toggle"
          class:on={enabled}
          aria-pressed={enabled}
          aria-label={$t('dev_sync_enable')}
          disabled={busy || !isTauri}
          onclick={toggle}
        ></button>
      </div>
      <div class="actions">
        <button type="button" class="btn btn-ghost btn-sm" disabled={!isTauri || entries.length === 0} onclick={clearLog}>
          {$t('dev_sync_clear')}
        </button>
        <button type="button" class="btn btn-ghost btn-sm" disabled={entries.length === 0} onclick={copyLog}>
          {copied ? $t('dev_sync_copied') : $t('dev_sync_copy')}
        </button>
      </div>
      {#if error}<p class="err">{error}</p>{/if}
    </div>

    <div class="log" bind:this={logEl} onscroll={onScroll}>
      {#if entries.length === 0}
        <p class="empty">{$t('dev_sync_empty')}</p>
      {:else}
        {#each entries as row (row.seq)}
          <div class="line" class:error={row.level === 'error'} class:retry={row.level === 'retry'} class:skip={row.level === 'skip'}>
            {lineText(row)}
          </div>
        {/each}
      {/if}
    </div>
  </div>
</div>

<style>
  .body { display: flex; flex-direction: column; gap: var(--sp-3); }
  .back {
    color: var(--text-faint);
    font-size: var(--fs-sm);
    text-decoration: none;
  }
  .back:hover { color: var(--text); text-decoration: underline; }
  .panel { display: flex; flex-direction: column; gap: var(--sp-3); padding: var(--sp-4); }
  .toggle-row { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-4); }
  .toggle-info { display: flex; flex-direction: column; gap: 0.2rem; }
  .hint { font-size: var(--fs-sm); color: var(--text-dim); }
  .actions { display: flex; gap: var(--sp-2); }
  .err { margin: 0; font-size: var(--fs-sm); color: var(--danger-text); }
  .log {
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 1.45;
    min-height: 12rem;
    flex-shrink: 0;
    max-height: min(70vh, 640px);
    overflow: auto;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: var(--sp-3);
  }
  .empty { margin: 0; color: var(--text-faint); }
  .line { white-space: pre-wrap; word-break: break-word; }
  .line.error { color: var(--danger-text); }
  .line.retry { color: var(--warn-text); }
  .line.skip { color: var(--text-faint); }
</style>
