<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- The Backup card of the settings page: the backup service of Space (platform-spec 1.2, 11.3). -->
<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { formatError, formatBytes } from '$lib/core/utils';
  import CustomSelect from '$lib/core/ui/CustomSelect.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import { api } from '$lib/backup/api';
  import type { BackupConfig, BackupFileInfo } from '$lib/backup/types';

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
      await api.backup.restore(restoreTarget.path, restorePassword);
    } catch (e) {
      restoring = false;
      restoreError = formatError(e);
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

  <!-- Destination folder -->
  <div class="field">
    <span class="field-label">{$t('settings_backup_folder')}</span>
    <div class="dir-row">
      <input
        class="dir-input"
        type="text"
        bind:value={backupCfg.dir}
        placeholder={$t('settings_backup_folder_placeholder')}
        readonly={!isTauri}
      />
      {#if isTauri}
        <button class="btn btn-ghost btn-sm btn-icon" onclick={browseBackupDir} title={$t('settings_backup_browse')}>
          <Icon name="folder-open" size={14} />
        </button>
      {/if}
    </div>
  </div>

  <!-- Password -->
  <div class="field">
    <span class="field-label">{$t('settings_backup_password')}</span>
    <input
      class="field-input"
      type="password"
      bind:value={backupCfg.password}
      placeholder={backupCfg.has_password && backupCfg.password == null ? $t('secret_stored_placeholder') : $t('settings_backup_password_placeholder')}
      autocomplete="off"
    />
    {#if backupCfg.has_password && backupCfg.password == null}
      <button type="button" class="link-btn" onclick={() => (backupCfg.password = '')}>{$t('secret_clear')}</button>
    {/if}
    <p class="muted small">{$t('settings_backup_password_note')}</p>
  </div>

  <!-- Schedule -->
  <div class="dev-tools-row">
    <div class="dev-tools-info">
      <span>{$t('settings_backup_schedule')}</span>
      <span class="muted">{$t('settings_backup_schedule_hint')}</span>
    </div>
    <button
      class="toggle"
      class:on={backupCfg.schedule_enabled}
      onclick={() => { backupCfg.schedule_enabled = !backupCfg.schedule_enabled; }}
      aria-pressed={backupCfg.schedule_enabled}
      aria-label={$t('settings_backup_schedule')}
    ></button>
  </div>

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
        <input class="dir-input" type="number" min="0" bind:value={backupCfg.keep} />
      </div>
    </div>
  {/if}

  <div class="btn-row">
    <button class="btn btn-primary btn-sm" disabled={backupSaving} onclick={saveBackupConfig}>
      {backupSaving ? $t('settings_backup_saving') : $t('settings_backup_save')}
    </button>
    <button
      class="btn btn-ghost btn-sm"
      disabled={!isTauri || backupRunning || !backupCfg.dir || !(backupCfg.password || (backupCfg.has_password && backupCfg.password == null))}
      onclick={runBackupNow}
    >
      {backupRunning ? $t('settings_backup_running') : $t('settings_backup_run_now')}
    </button>
    {#if backupSaved}<span class="ok-msg">✓</span>{/if}
  </div>

  {#if backupRunning}
    <div class="progress-wrap">
      <div class="progress-bar">
        <div class="progress-fill" style="width: {backupProgress}%"></div>
      </div>
      <span class="progress-label">{backupPhase} · {backupProgress}%</span>
    </div>
  {/if}
  {#if backupDone}<p class="ok-msg">{backupDone}</p>{/if}
  {#if backupError}<div class="error-msg">{backupError}</div>{/if}

  <!-- Existing backups -->
  <div class="field">
    <span class="field-label">{$t('settings_backup_existing')}</span>
    {#if backupList.length === 0}
      <p class="muted small">{$t('settings_backup_none')}</p>
    {:else}
      <div class="backup-list">
        {#each backupList as b (b.path)}
          <div class="backup-item">
            <div class="backup-meta">
              <span class="backup-name">{b.name}</span>
              <span class="muted small">{formatBytes(b.size)}</span>
            </div>
            {#if isTauri}
              <button class="btn btn-ghost btn-sm" onclick={() => openRestore(b)}>
                {$t('settings_backup_restore')}
              </button>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </div>

  <div class="version-table">
    <div class="version-row">
      <span class="version-label">{$t('settings_backup_last_run')}</span>
      <span class="version-value">
        {backupCfg.last_run ? new Date(backupCfg.last_run).toLocaleString() : $t('settings_backup_never')}
      </span>
    </div>
  </div>

{#if restoreTarget}
  <Dialog open={true} onclose={closeRestore} title={$t('settings_backup_restore_title')}>
    <div class="restore-body">
      <p class="backup-name">{restoreTarget.name}</p>
      <p class="warn-msg">{$t('settings_backup_restore_warn')}</p>
      <input
        class="field-input"
        type="password"
        bind:value={restorePassword}
        placeholder={backupCfg.has_password ? $t('secret_stored_placeholder') : $t('settings_backup_password')}
        autocomplete="off"
        disabled={restoring}
      />
      {#if restoring}
        <div class="progress-wrap">
          <div class="progress-bar">
            <div class="progress-fill" style="width: {restorePercent}%"></div>
          </div>
          <span class="progress-label">{restorePhase} · {restorePercent}%</span>
        </div>
      {/if}
      {#if restoreError}<div class="error-msg">{restoreError}</div>{/if}
    </div>
    {#snippet footer()}
      <button class="btn btn-ghost btn-sm" disabled={restoring} onclick={closeRestore}>
        {$t('settings_backup_cancel')}
      </button>
      <button class="btn btn-danger btn-sm" disabled={(!restorePassword && !backupCfg.has_password) || restoring} onclick={confirmRestore}>
        {restoring ? $t('settings_backup_restoring') : $t('settings_backup_restore_confirm')}
      </button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .link-btn {
    align-self: flex-start; display: inline-flex; align-items: center; gap: 0.4rem;
    background: none; border: none; padding: 0; cursor: pointer;
    color: var(--accent-text-3); font-size: var(--fs-sm); font-weight: 500; text-decoration: underline;
  }
  .link-btn:hover { color: var(--accent-text); }

  .sched-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: var(--sp-3);
  }

  .backup-list { display: flex; flex-direction: column; gap: var(--sp-2); }
  .backup-item {
    display: flex; align-items: center; justify-content: space-between; gap: var(--sp-3);
    padding: var(--sp-2) var(--sp-3);
    background: var(--surface-3); border: 1px solid var(--border); border-radius: var(--radius);
  }
  .backup-meta { display: flex; flex-direction: column; gap: 0.1rem; min-width: 0; }
  .backup-name {
    font-size: var(--fs-sm); color: var(--text-body); font-family: var(--font-mono);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }

  .restore-body { display: flex; flex-direction: column; gap: var(--sp-3); }
</style>
