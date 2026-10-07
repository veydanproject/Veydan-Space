<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { syncStore } from '$lib/core/store/sync.svelte';
  import { notesStore } from '$lib/notes/store/notes.svelte';
  import { api } from '$lib/notes/api';
  import { t, locale } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { formatDateTime } from '$lib/core/utils';

  onMount(() => {
    let off: (() => void) | null = null;
    void syncStore.listen().then((fn) => (off = fn));
    return () => off?.();
  });

  const running = $derived(syncStore.busy || syncStore.status?.running === true);
  const progress = $derived(syncStore.progress);

  const title = $derived.by(() => {
    const s = syncStore.status;
    const p = progress;
    if (running && p) {
      const phase = $t(`settings_sync_phase_${p.phase}` as 'settings_sync_phase_collect');
      const line = p.total > 0
        ? $t('settings_sync_progress_files', { phase, current: String(p.current), total: String(p.total), percent: String(p.percent) })
        : $t('settings_sync_progress', { phase, percent: String(p.percent) });
      return p.detail ? `${line}\n${p.detail}` : line;
    }
    if (running) return $t('notes_sync_running');
    if (!s?.joined) return `${$t('notes_btn_sync')} · ${$t('notes_sync_not_joined')}`;
    const last = s.last_run ? formatDateTime(s.last_run, $locale) : $t('notes_sync_never');
    const err = syncStore.error || s.last_error;
    const warn = s.last_warning;
    let base = `${$t('notes_btn_sync')} · ${$t('notes_sync_last')}: ${last}`;
    if (s.last_applied !== null) base += ` · ${$t('notes_sync_applied', { n: String(s.last_applied) })}`;
    if (err) base += `\n${$t('notes_sync_error')}: ${err}`;
    if (warn) base += `\n${$t('settings_sync_last_warning')}: ${warn}`;
    return base;
  });

  const hasError = $derived(!running && !!(syncStore.error || syncStore.status?.last_error));

  async function run() {
    // Without a vault the button reindexes the notes folder.
    await syncStore.runNow(() => api.notes.sync());
    await notesStore.refresh();
  }
</script>

<button class="icon-btn" class:danger={hasError} {title} disabled={running} onclick={run}>
  <span class:spin={running}><Icon name="refresh-cw" size={14} /></span>
  {#if running && progress}<span class="pct">{progress.percent}</span>{/if}
</button>

<style>
  span { display: inline-flex; }
  .pct { font-size: 0.65rem; font-variant-numeric: tabular-nums; color: var(--text-dim); min-width: 1.4em; }
</style>
