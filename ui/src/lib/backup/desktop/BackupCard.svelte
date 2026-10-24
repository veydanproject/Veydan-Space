<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The Backup card of the settings page: the backup service of Space (platform-spec 1.2, 11.3). -->
<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { t, locale } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { formatDateTime, formatError, formatBytes } from '$lib/core/utils';
  import CustomSelect from '$lib/core/ui/CustomSelect.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { api, olderBackup } from '$lib/backup/api';
  import type { BackupConfig, BackupFileInfo, RestoreOutcome } from '$lib/backup/types';

  const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

  let unlisteners: (() => void)[] = [];

  // Backup
  let backupCfg = $state<BackupConfig>({
    dir: null,
    password: null,
    has_password: false,
    schedule_enabled: false,
    schedule_mode: 'interval',
    interval_hours: 24,
    time: '03:00',
    weekday: 0,
    keep: 5,
    last_run: null,
    last_restore: null,
  });
  let backupList = $state<BackupFileInfo[]>([]);
  let backupSaving = $state(false);
  let backupSaved = $state(false);
  let backupError = $state('');
  let backupRunning = $state(false);
  let backupPhase = $state('');
  let backupProgress = $state(0);
  let backupDone = $state('');
  // Restore dialog
  let restoreTarget = $state<BackupFileInfo | null>(null);
  let restorePassword = $state('');
  let restoreError = $state('');
  let restoring = $state(false);
  let restorePhase = $state('');
  let restorePercent = $state(0);
  /** The device is joined to a vault: the restore makes it leave (backup_restore). */
  let restoreJoined = $state(false);
  /** The archive is older than the newest one of the data: the dates, shown before it is restored anyway. */
  let restoreOlder = $state<{ created: string; newest: string } | null>(null);

  const modeOptions = $derived([
    { value: 'interval', label: $t('settings_backup_mode_interval') },
    { value: 'daily', label: $t('settings_backup_mode_daily') },
    { value: 'weekly', label: $t('settings_backup_mode_weekly') },
  ]);
  const weekdayOptions = $derived(
    [0, 1, 2, 3, 4, 5, 6].map((d) => ({
      value: String(d),
      label: $t(`settings_backup_day_${d}` as any),
    }))
  );

  function restoreOutcomeText(o: RestoreOutcome): string {
    switch (o.result) {
      case 'restored':
        return $t('settings_backup_restore_result_restored');
      case 'failed':
        return $t('settings_backup_restore_result_failed', { error: o.error ?? '' });
      case 'undone':
        return $t('settings_backup_restore_result_undone');
      default:
        return $t('settings_backup_restore_result_finished');
    }
  }

  async function loadBackup() {
    try {
      backupCfg = await api.backup.getConfig();
    } catch {}
    await refreshBackupList();
  }

  async function refreshBackupList() {
    try {
      backupList = await api.backup.list();
    } catch {
      backupList = [];
    }
  }

  async function browseBackupDir() {
    if (!isTauri) return;
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const selected = await open({ directory: true, multiple: false, title: $t('settings_backup_folder') });
      if (selected && typeof selected === 'string') {
        backupCfg.dir = selected;
      }
    } catch {}
  }

  async function saveBackupConfig() {
    backupSaving = true;
    backupSaved = false;
    backupError = '';
    try {
      await api.backup.setConfig($state.snapshot(backupCfg));
      backupSaved = true;
      setTimeout(() => (backupSaved = false), 2000);
      await refreshBackupList();
    } catch (e) {
      backupError = formatError(e);
    } finally {
      backupSaving = false;
    }
  }

  async function runBackupNow() {
    backupError = '';
    backupDone = '';
    // Persist current config first so the backend uses the latest dir/password.
    await saveBackupConfig();
    if (backupError) return;
    backupRunning = true;
    backupProgress = 0;
    backupPhase = '';
    try {
      await api.backup.runNow();
    } catch (e) {
      backupRunning = false;
      backupError = formatError(e);
    }
  }

  function openRestore(b: BackupFileInfo) {
    restoreTarget = b;
    // Empty = the backend uses the stored backup password.
    restorePassword = '';
    restoreError = '';
    restoreJoined = false;
    restoreOlder = null;
    void api.sync.status().then((s) => (restoreJoined = s.joined)).catch(() => {});
  }

  function closeRestore() {
    if (restoring) return;
    restoreTarget = null;
  }

  async function confirmRestore() {
    if (!restoreTarget) return;
    restoreError = '';
    restoring = true;
    restorePhase = '';
    restorePercent = 0;
    try {
      // On success the app restarts, so this call never resolves.
      await api.backup.restore(restoreTarget.path, restorePassword, restoreOlder !== null);
    } catch (e) {
      restoring = false;
      const older = olderBackup(formatError(e));
      if (older) restoreOlder = older;
      else restoreError = formatError(e);
    }
  }

  onMount(async () => {
    await loadBackup();

    if (isTauri) {
      const { listen } = await import('@tauri-apps/api/event');

      unlisteners.push(await listen<{ phase: string; percent: number }>('backup://progress', (e) => {
        backupRunning = true;
        backupPhase = e.payload.phase;
        backupProgress = e.payload.percent;
      }));

      unlisteners.push(await listen<string>('backup://done', async () => {
        backupRunning = false;
        backupProgress = 100;
        backupDone = $t('settings_backup_done');
        setTimeout(() => (backupDone = ''), 4000);
        await Promise.all([refreshBackupList(), (async () => { backupCfg = await api.backup.getConfig().catch(() => backupCfg); })()]);
      }));

      unlisteners.push(await listen<{ phase: string; percent: number }>('backup://restore-progress', (e) => {
        restorePhase = e.payload.phase;
        restorePercent = e.payload.percent;
      }));

      unlisteners.push(await listen<string>('backup://error', (e) => {
        backupRunning = false;
        backupError = e.payload;
      }));
    }
  });

  onDestroy(() => unlisteners.forEach((fn) => fn()));
</script>

<!-- Where and with what: the folder and the password. The schedule: its switch row and the fields it
     governs. Then the progress, the outcome, the actions; the existing backups in a card of their own. -->
<div class="backup">
  <div class="card where">
    <div class="field">
      <span class="field-label">{$t('settings_backup_folder')}</span>
      <div class="path-row">
        <input
          class="dir-input"
          type="text"
          bind:value={backupCfg.dir}
          placeholder={$t('settings_backup_folder_placeholder')}
          aria-label={$t('settings_backup_folder')}
          readonly={!isTauri}
        />
        {#if isTauri}
          <button type="button" class="btn btn-ghost browse" onclick={browseBackupDir}>
            <Icon name="folder-open" size={18} />
            {$t('settings_backup_browse')}
          </button>
        {/if}
      </div>
    </div>

    <div class="field">
      <span class="field-label">{$t('settings_backup_password')}</span>
      <input
        class="field-input"
        type="password"
        bind:value={backupCfg.password}
        placeholder={backupCfg.has_password && backupCfg.password == null ? $t('secret_stored_placeholder') : $t('settings_backup_password_placeholder')}
        aria-label={$t('settings_backup_password')}
        autocomplete="off"
      />
      {#if backupCfg.has_password && backupCfg.password == null}
        <button type="button" class="link-btn" onclick={() => (backupCfg.password = '')}>{$t('secret_clear')}</button>
      {/if}
      <p class="note">{$t('settings_backup_password_note')}</p>
    </div>
  </div>

  <div class="card schedule">
    <button
      type="button"
      class="switch-row"
      role="switch"
      aria-checked={backupCfg.schedule_enabled}
      onclick={() => { backupCfg.schedule_enabled = !backupCfg.schedule_enabled; }}
    >
      <span class="words">
        <span class="title">{$t('settings_backup_schedule')}</span>
        <span class="note">{$t('settings_backup_schedule_hint')}</span>
      </span>
      <span class="toggle" class:on={backupCfg.schedule_enabled} aria-hidden="true"></span>
    </button>

    {#if backupCfg.schedule_enabled}
      <div class="sched-grid">
        <div class="field">
          <span class="field-label">{$t('settings_backup_mode')}</span>
          <CustomSelect
            options={modeOptions}
            value={backupCfg.schedule_mode}
            onchange={(v) => (backupCfg.schedule_mode = (v as BackupConfig['schedule_mode']) ?? 'interval')}
          />
        </div>

        {#if backupCfg.schedule_mode === 'interval'}
          <div class="field">
            <span class="field-label">{$t('settings_backup_interval_hours')}</span>
            <input class="field-input" type="number" min="1" bind:value={backupCfg.interval_hours} />
          </div>
        {:else}
          <div class="field">
            <span class="field-label">{$t('settings_backup_time')}</span>
            <input class="field-input" type="time" bind:value={backupCfg.time} />
          </div>
          {#if backupCfg.schedule_mode === 'weekly'}
            <div class="field">
              <span class="field-label">{$t('settings_backup_weekday')}</span>
              <CustomSelect
                options={weekdayOptions}
                value={String(backupCfg.weekday)}
                onchange={(v) => (backupCfg.weekday = Number(v ?? 0))}
              />
            </div>
          {/if}
        {/if}

        <div class="field">
          <span class="field-label">{$t('settings_backup_keep')}</span>
          <input class="field-input" type="number" min="0" bind:value={backupCfg.keep} />
        </div>
      </div>
    {/if}
  </div>

  {#if backupRunning}
    <div class="progress-wrap" role="status">
      <div class="progress-bar">
        <div class="progress-fill" style="width: {backupProgress}%"></div>
      </div>
      <span class="progress-label">{backupPhase} · {backupProgress}%</span>
    </div>
  {/if}
  {#if backupDone}<div class="done" role="status">{backupDone}</div>{/if}
  {#if backupError}
    <div class="fail" role="alert"><Icon name="alert-triangle" size={18} /><span>{backupError}</span></div>
  {/if}

  <div class="foot">
    {#if backupSaved}<span class="saved" role="status">{$t('settings_backup_saved')}</span>{/if}
    <button
      type="button"
      class="btn btn-ghost"
      disabled={!isTauri || backupRunning || !backupCfg.dir || !(backupCfg.password || (backupCfg.has_password && backupCfg.password == null))}
      onclick={runBackupNow}
    >
      {backupRunning ? $t('settings_backup_running') : $t('settings_backup_run_now')}
    </button>
    <button type="button" class="btn btn-primary" disabled={backupSaving} onclick={saveBackupConfig}>
      {backupSaving ? $t('settings_backup_saving') : $t('settings_backup_save')}
    </button>
  </div>

  <section class="group" aria-labelledby="backup-existing">
    <div class="caption" id="backup-existing">{$t('settings_backup_existing')}</div>
    <div class="card list">
      <div class="item fact">
        <span class="fact-key">{$t('settings_backup_last_run')}</span>
        <span class="fact-value">{backupCfg.last_run ? formatDateTime(backupCfg.last_run, $locale) : $t('settings_backup_never')}</span>
      </div>
      {#if backupCfg.last_restore}
        <div class="item fact">
          <span class="fact-key">{$t('settings_backup_last_restore')}</span>
          <span class="fact-value">
            {formatDateTime(backupCfg.last_restore.at, $locale)} · {restoreOutcomeText(backupCfg.last_restore)}
            {#each backupCfg.last_restore.kept ?? [] as path (path)}
              <br />{$t('settings_backup_restore_kept', { path })}
            {/each}
          </span>
        </div>
      {/if}
      {#if backupList.length === 0}
        <p class="item none">{$t('settings_backup_none')}</p>
      {:else}
        {#each backupList as b (b.path)}
          <div class="item file">
            <span class="file-words">
              <span class="backup-name">{b.name}</span>
              <span class="note">{formatBytes(b.size)}</span>
            </span>
            {#if isTauri}
              <button type="button" class="btn btn-ghost" onclick={() => openRestore(b)}>
                {$t('settings_backup_restore')}
              </button>
            {/if}
          </div>
        {/each}
      {/if}
    </div>
  </section>
</div>

{#if restoreTarget}
  <Dialog open={true} onclose={closeRestore} title={$t('settings_backup_restore_title')}>
    <div class="restore-body">
      <span class="backup-name file-chip">{restoreTarget.name}</span>
      <div class="caution"><Icon name="alert-triangle" size={18} /><span>{$t('settings_backup_restore_warn')}</span></div>
      {#if restoreJoined}<div class="caution"><Icon name="alert-triangle" size={18} /><span>{$t('settings_backup_restore_sync')}</span></div>{/if}
      {#if restoreOlder}
        <div class="caution">
          <Icon name="alert-triangle" size={18} />
          <span>
            {$t('settings_backup_restore_older', {
              created: formatDateTime(restoreOlder.created, $locale),
              newest: formatDateTime(restoreOlder.newest, $locale),
            })}
          </span>
        </div>
      {/if}
      <input
        class="field-input"
        type="password"
        bind:value={restorePassword}
        placeholder={backupCfg.has_password ? $t('secret_stored_placeholder') : $t('settings_backup_password')}
        aria-label={$t('settings_backup_password')}
        autocomplete="off"
        disabled={restoring}
      />
      {#if restoring}
        <div class="progress-wrap" role="status">
          <div class="progress-bar">
            <div class="progress-fill" style="width: {restorePercent}%"></div>
          </div>
          <span class="progress-label">{restorePhase} · {restorePercent}%</span>
        </div>
      {/if}
      {#if restoreError}
        <div class="fail" role="alert"><Icon name="alert-triangle" size={18} /><span>{restoreError}</span></div>
      {/if}
    </div>
    {#snippet footer()}
      <button type="button" class="btn btn-ghost" disabled={restoring} onclick={closeRestore}>
        {$t('settings_backup_cancel')}
      </button>
      <button type="button" class="btn btn-danger" disabled={(!restorePassword && !backupCfg.has_password) || restoring} onclick={confirmRestore}>
        {restoring
          ? $t('settings_backup_restoring')
          : restoreOlder
            ? $t('settings_backup_restore_anyway')
            : $t('settings_backup_restore_confirm')}
      </button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .backup { display: flex; flex-direction: column; gap: var(--sp-4); min-width: 0; container-type: inline-size; }
  .card.where { padding: var(--sp-4); display: flex; flex-direction: column; gap: var(--sp-4); }
  .path-row { display: flex; align-items: center; gap: var(--sp-2); }
  .browse { height: var(--control-h-lg); flex-shrink: 0; }
  .field-input { height: var(--control-h-lg); }
  .note { margin: 0; font-size: var(--fs-sm); line-height: 1.4; color: var(--text-2); }
  .link-btn {
    align-self: flex-start; display: inline-flex; align-items: center; gap: 0.4rem;
    background: none; border: none; padding: 0; cursor: pointer;
    color: var(--accent-text-3); font-size: var(--fs-sm); font-weight: 500; text-decoration: underline;
  }
  .link-btn:hover { color: var(--accent-text); }

  .card.schedule { padding: var(--sp-1); }
  .switch-row {
    display: flex; align-items: center; gap: var(--sp-3); width: 100%; min-height: 64px; padding: var(--sp-2) var(--sp-4);
    border: 0; border-radius: 0; background: none; color: var(--text); font: inherit; text-align: left; cursor: pointer;
    transition: background var(--dur-fast);
  }
  /* The hover of a row: square between lines, the first and the last row take the card's corners (its radius less its padding). */
  .switch-row:first-child { border-top-left-radius: calc(var(--radius-card) - var(--sp-1)); border-top-right-radius: calc(var(--radius-card) - var(--sp-1)); }
  .switch-row:last-child { border-bottom-left-radius: calc(var(--radius-card) - var(--sp-1)); border-bottom-right-radius: calc(var(--radius-card) - var(--sp-1)); }
  .switch-row:hover { background: var(--surface-row-hover); }
  .switch-row .toggle { pointer-events: none; }
  .words { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .title { font-size: var(--fs-body); font-weight: var(--fw-semibold); }
  .sched-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(170px, 1fr));
    gap: var(--sp-4);
    padding: var(--sp-3) var(--sp-4) var(--sp-4);
    border-top: 1px solid var(--divider);
  }

  .done {
    padding: var(--sp-3) var(--sp-4); border-radius: var(--radius-md);
    background: var(--success-bg); color: var(--success-text); font-size: var(--fs-base);
  }
  .fail {
    display: flex; align-items: flex-start; gap: var(--sp-3); padding: var(--sp-3) var(--sp-4);
    border-radius: var(--radius-md); background: var(--danger-bg); color: var(--text); font-size: var(--fs-base); line-height: 1.5;
    overflow-wrap: anywhere;
  }
  .fail :global(svg) { flex-shrink: 0; margin-top: 2px; color: var(--danger-text); }
  .foot { display: flex; justify-content: flex-end; align-items: center; flex-wrap: wrap; gap: var(--sp-2); }
  .saved { margin-right: var(--sp-1); font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--success-text); }

  .group { display: flex; flex-direction: column; gap: var(--sp-2); }
  .caption { padding: 0 var(--sp-1); font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text-2); }
  .card.list { padding: var(--sp-1); display: flex; flex-direction: column; }
  .item { position: relative; display: flex; align-items: center; gap: var(--sp-4); min-height: 52px; padding: var(--sp-2) var(--sp-4); margin: 0; }
  /* A straight line between rows: a shadow would follow the rounded corners. */
  .item + .item::before { content: ''; position: absolute; top: 0; left: 0; right: 0; height: 1px; background: var(--divider); }
  .fact-key { flex: 0 0 220px; font-size: var(--fs-sm); color: var(--text-dim); }
  .fact-value { flex: 1; min-width: 0; font-size: var(--fs-base); color: var(--text); overflow-wrap: anywhere; }
  .none { font-size: var(--fs-base); color: var(--text-2); }
  .file { min-height: 60px; }
  .file-words { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .backup-name {
    font-size: var(--fs-sm); color: var(--text-body); font-family: var(--font-mono);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }

  .restore-body { display: flex; flex-direction: column; gap: var(--sp-3); }
  .file-chip { align-self: flex-start; max-width: 100%; padding: var(--sp-1) var(--sp-3); border-radius: var(--radius-sm); background: var(--surface-2); }
  .caution {
    display: flex; gap: var(--sp-3); align-items: flex-start; padding: var(--sp-3) var(--sp-4);
    border-radius: var(--radius-md); background: var(--warn-bg); color: var(--warn-text);
    font-size: var(--fs-sm); line-height: 1.5;
  }
  .caution :global(svg) { flex-shrink: 0; margin-top: 1px; }

  /* A narrow window: a fact's name over its value. */
  @container (max-width: 560px) {
    .fact { flex-direction: column; align-items: flex-start; gap: 2px; }
    .fact-key { flex-basis: auto; }
  }
</style>
