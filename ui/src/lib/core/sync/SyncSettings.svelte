<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { t } from '$lib/core/i18n';
  import { api, LARGE_FILE_LIMITS, largeFilePeakMib } from '$lib/core/api';
  import type { SyncConfig, SyncStatus } from '$lib/core/api';
  import { syncStore } from '$lib/core/store/sync.svelte';
  import { formatError } from '$lib/core/utils';
  import { joinErrorKey } from '$lib/core/join-error';
  import CustomSelect from '$lib/core/ui/CustomSelect.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { directory } from '$lib/core/directory';
  import { registry } from '$lib/core/registry';

  const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

  let cfg = $state<SyncConfig>({
    enabled: false,
    backend: 'folder',
    folder_path: '',
    s3: { endpoint: '', region: '', bucket: '', prefix: '', access_key: '', has_secret_key: false, path_style: true },
    webdav: { url: '', username: '', has_password: false },
    interval_sec: 60,
    profile_files: true,
    device_name: '',
    large_files: { chunk_mib: 8, parallelism: 3, resume: true },
  });
  const [chunkMin, chunkMax] = LARGE_FILE_LIMITS.chunkMib;
  const [parMin, parMax] = LARGE_FILE_LIMITS.parallelism;
  const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, Math.floor(v || lo)));
  const peakMib = $derived(largeFilePeakMib(cfg.large_files));
  let status = $state<SyncStatus | null>(null);
  let devicesOpen = $state(false);
  let passphrase = $state('');
  let oldPassphrase = $state('');
  let newPassphrase = $state('');
  let busy = $state(false);
  let saved = $state(false);
  let error = $state('');
  let info = $state('');
  let confirmLeave = $state(false);
  let unlisten: (() => void) | null = null;

  const backendOptions = $derived([
    { value: 'folder', label: $t('settings_sync_backend_folder') },
    { value: 's3', label: $t('settings_sync_backend_s3') },
    { value: 'webdav', label: $t('settings_sync_backend_webdav') },
  ]);

  const MAX_INTERVAL_SEC = 86400;
  // Show minutes when the stored value is a whole number of minutes.
  let intervalUnit = $state<'sec' | 'min'>('min');
  const intervalValue = $derived(intervalUnit === 'min' ? Math.max(1, Math.round(cfg.interval_sec / 60)) : cfg.interval_sec);
  const unitOptions = $derived([
    { value: 'sec', label: $t('settings_sync_unit_seconds') },
    { value: 'min', label: $t('settings_sync_unit_minutes') },
  ]);
  const presets = [1, 5, 10, 30, 60, 300, 900, 3600];
  const remoteFastWarn = $derived(cfg.backend !== 'folder' && cfg.interval_sec < 60);
  const progress = $derived(syncStore.progress);
  const progressText = $derived.by(() => {
    const p = progress;
    if (!p) return '';
    const key = `settings_sync_phase_${p.phase}` as 'settings_sync_phase_collect';
    const phase = $t(key);
    if (p.total > 0) {
      return $t('settings_sync_progress_files', {
        phase,
        current: String(p.current),
        total: String(p.total),
        percent: String(p.percent),
      });
    }
    return $t('settings_sync_progress', { phase, percent: String(p.percent) });
  });

  function unitFor(sec: number): 'sec' | 'min' {
    return sec >= 60 && sec % 60 === 0 ? 'min' : 'sec';
  }

  function setIntervalSec(sec: number) {
    cfg.interval_sec = Math.min(MAX_INTERVAL_SEC, Math.max(1, Math.floor(sec || 1)));
  }

  function setIntervalValue(v: number) {
    setIntervalSec(intervalUnit === 'min' ? v * 60 : v);
  }

  function setIntervalUnit(unit: 'sec' | 'min') {
    if (unit === intervalUnit) return;
    // Keep the number, change the meaning: 5 min -> 5 s and back.
    const n = intervalValue;
    intervalUnit = unit;
    setIntervalSec(unit === 'min' ? n * 60 : n);
  }

  function applyPreset(sec: number) {
    intervalUnit = unitFor(sec);
    setIntervalSec(sec);
  }

  /** Short forms ("1 s", "5 min"): a number with a unit word needs a plural form per count. */
  function presetLabel(sec: number): string {
    return sec >= 60 ? $t('settings_sync_interval_min', { n: String(sec / 60) }) : $t('settings_sync_interval_sec', { n: String(sec) });
  }

  // Browser profiles sync their files and hold a lease while they run: both
  // are shown only in a product whose module owns the profiles.
  const hasProfiles = $derived(directory.owned('profile'));
  /** Large-file transfer applies where a module syncs files (notes' attachments, profile files). */
  const syncsFiles = registry.modules.some((m) => m.syncsFiles);

  async function refreshStatus() {
    try {
      status = await api.sync.status();
    } catch {}
  }

  async function run(task: () => Promise<void>) {
    busy = true;
    error = '';
    info = '';
    try {
      await task();
    } catch (e) {
      const key = joinErrorKey(e);
      error = key ? $t(key) : formatError(e);
    } finally {
      busy = false;
      await refreshStatus();
    }
  }

  async function saveConfig() {
    await api.sync.setConfig($state.snapshot(cfg));
    cfg = await api.sync.getConfig();
    saved = true;
    setTimeout(() => (saved = false), 2000);
  }

  const save = () => run(saveConfig);

  const probe = () =>
    run(async () => {
      await saveConfig();
      const result = await api.sync.probe();
      info = $t(`settings_sync_probe_${result}` as any);
    });

  const createVault = () =>
    run(async () => {
      await saveConfig();
      status = await api.sync.createVault(passphrase);
      // The backend turned sync on: the form takes its config, or the next save would turn it off.
      cfg = await api.sync.getConfig();
      passphrase = '';
      info = $t('settings_sync_created');
    });

  const joinVault = () =>
    run(async () => {
      await saveConfig();
      status = await api.sync.joinVault(passphrase);
      cfg = await api.sync.getConfig();
      passphrase = '';
      info = $t('settings_sync_joined');
    });

  const syncNow = () =>
    run(async () => {
      await saveConfig();
      status = await api.sync.runNow();
      info = $t('settings_sync_done');
    });

  const leave = () =>
    run(async () => {
      confirmLeave = false;
      status = await api.sync.leave();
      cfg = await api.sync.getConfig();
      info = $t('settings_sync_left');
    });

  const changePassphrase = () =>
    run(async () => {
      await api.sync.changePassphrase(oldPassphrase, newPassphrase);
      oldPassphrase = '';
      newPassphrase = '';
      info = $t('settings_sync_passphrase_changed');
    });

  async function browseFolder() {
    if (!isTauri) return;
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const selected = await open({ directory: true, multiple: false, title: $t('settings_sync_folder') });
      if (selected && typeof selected === 'string') cfg.folder_path = selected;
    } catch {}
  }

  onMount(async () => {
    try {
      cfg = await api.sync.getConfig();
      intervalUnit = unitFor(cfg.interval_sec);
    } catch {}
    await refreshStatus();
    if (isTauri) {
      const { listen } = await import('@tauri-apps/api/event');
      unlisten = await listen('sync://status', () => void refreshStatus());
      void syncStore.listen().then((fn) => {
        const prev = unlisten;
        unlisten = () => {
          prev?.();
          fn();
        };
      });
    }
  });

  onDestroy(() => unlisten?.());
</script>

<div class="sync">
  <div class="field">
    <span class="field-label">{$t('settings_sync_backend')}</span>
    <CustomSelect options={backendOptions} value={cfg.backend} onchange={(v) => (cfg.backend = v ?? 'folder')} />
  </div>

  {#if cfg.backend === 'folder'}
    <div class="field">
      <span class="field-label">{$t('settings_sync_folder')}</span>
      <div class="row">
        <input class="input mono" type="text" bind:value={cfg.folder_path} placeholder={$t('settings_sync_folder_placeholder')} readonly={!isTauri} />
        {#if isTauri}
          <button class="btn btn-ghost btn-sm icon" onclick={browseFolder} title={$t('settings_backup_browse')}>
            <Icon name="folder-open" size={14} />
          </button>
        {/if}
      </div>
      <p class="hint">{$t('settings_sync_folder_hint')}</p>
    </div>
  {:else if cfg.backend === 's3'}
    <div class="grid">
      <label class="field"><span class="field-label">{$t('settings_sync_s3_endpoint')}</span><input class="input" type="text" bind:value={cfg.s3.endpoint} placeholder="https://s3.example.com" /></label>
      <label class="field"><span class="field-label">{$t('settings_sync_s3_region')}</span><input class="input" type="text" bind:value={cfg.s3.region} placeholder="us-east-1" /></label>
      <label class="field"><span class="field-label">{$t('settings_sync_s3_bucket')}</span><input class="input" type="text" bind:value={cfg.s3.bucket} /></label>
      <label class="field"><span class="field-label">{$t('settings_sync_s3_prefix')}</span><input class="input" type="text" bind:value={cfg.s3.prefix} placeholder="veydan" /></label>
      <label class="field"><span class="field-label">{$t('settings_sync_s3_access_key')}</span><input class="input" type="text" bind:value={cfg.s3.access_key} autocomplete="off" /></label>
      <label class="field"><span class="field-label">{$t('settings_sync_s3_secret_key')}</span><input class="input" type="password" bind:value={cfg.s3.secret_key} placeholder={cfg.s3.has_secret_key && cfg.s3.secret_key == null ? $t('secret_stored_placeholder') : ''} autocomplete="off" /></label>
    </div>
    <div class="toggle-row">
      <div class="toggle-info">
        <span>{$t('settings_sync_s3_path_style')}</span>
        <span class="hint">{$t('settings_sync_s3_path_style_hint')}</span>
      </div>
      <button class="toggle" class:on={cfg.s3.path_style} onclick={() => (cfg.s3.path_style = !cfg.s3.path_style)} aria-pressed={cfg.s3.path_style} aria-label={$t('settings_sync_s3_path_style')}></button>
    </div>
  {:else}
    <div class="grid">
      <label class="field wide"><span class="field-label">{$t('settings_sync_webdav_url')}</span><input class="input" type="text" bind:value={cfg.webdav.url} placeholder="https://webdav.example.com/veydan" /></label>
      <label class="field"><span class="field-label">{$t('settings_sync_webdav_username')}</span><input class="input" type="text" bind:value={cfg.webdav.username} autocomplete="off" /></label>
      <label class="field"><span class="field-label">{$t('settings_sync_webdav_password')}</span><input class="input" type="password" bind:value={cfg.webdav.password} placeholder={cfg.webdav.has_password && cfg.webdav.password == null ? $t('secret_stored_placeholder') : ''} autocomplete="off" /></label>
    </div>
  {/if}

  <div class="field">
    <span class="field-label">{$t('settings_sync_interval')}</span>
    <div class="row">
      <input class="input interval" type="number" min="1" max={intervalUnit === 'min' ? MAX_INTERVAL_SEC / 60 : MAX_INTERVAL_SEC} value={intervalValue} oninput={(e) => setIntervalValue(Number((e.currentTarget as HTMLInputElement).value))} />
      <CustomSelect options={unitOptions} value={intervalUnit} onchange={(v) => setIntervalUnit(v === 'sec' ? 'sec' : 'min')} />
    </div>
    <div class="presets">
      <span class="hint">{$t('settings_sync_interval_presets')}</span>
      {#each presets as sec (sec)}
        <button class="chip" class:on={cfg.interval_sec === sec} onclick={() => applyPreset(sec)}>{presetLabel(sec)}</button>
      {/each}
    </div>
    {#if remoteFastWarn}
      <p class="warn">{$t('settings_sync_interval_remote_warn')}</p>
    {/if}
  </div>

  {#if hasProfiles}
    <div class="toggle-row">
      <div class="toggle-info">
        <span>{$t('settings_sync_profile_files')}</span>
        <span class="hint">{$t('settings_sync_profile_files_hint')}</span>
      </div>
      <button class="toggle" class:on={cfg.profile_files} onclick={() => (cfg.profile_files = !cfg.profile_files)} aria-pressed={cfg.profile_files} aria-label={$t('settings_sync_profile_files')}></button>
    </div>
  {/if}

  <label class="field">
    <span class="field-label">{$t('settings_sync_device_name')}</span>
    <input class="input" type="text" bind:value={cfg.device_name} maxlength="64" />
    <span class="hint">{hasProfiles ? $t('settings_sync_device_name_profiles_hint') : $t('settings_sync_device_name_hint')}</span>
  </label>

  {#if syncsFiles}
  <div class="field">
    <span class="field-label">{$t('settings_sync_lf_section')}</span>
    <p class="hint">{$t('settings_sync_lf_hint')}</p>
    <div class="grid">
      <label class="field">
        <span class="field-label">{$t('settings_sync_lf_chunk')}</span>
        <input class="input" type="number" min={chunkMin} max={chunkMax} value={cfg.large_files.chunk_mib}
          oninput={(e) => (cfg.large_files.chunk_mib = clamp(Number((e.currentTarget as HTMLInputElement).value), chunkMin, chunkMax))} />
        <span class="hint">{$t('settings_sync_lf_chunk_hint', { min: String(chunkMin), max: String(chunkMax) })}</span>
      </label>
      <label class="field">
        <span class="field-label">{$t('settings_sync_lf_parallelism')}</span>
        <input class="input" type="number" min={parMin} max={parMax} value={cfg.large_files.parallelism}
          oninput={(e) => (cfg.large_files.parallelism = clamp(Number((e.currentTarget as HTMLInputElement).value), parMin, parMax))} />
        <span class="hint">{$t('settings_sync_lf_parallelism_hint', { min: String(parMin), max: String(parMax) })}</span>
      </label>
    </div>
    <div class="toggle-row">
      <div class="toggle-info">
        <span>{$t('settings_sync_lf_resume')}</span>
        <span class="hint">{$t('settings_sync_lf_resume_hint')}</span>
      </div>
      <button class="toggle" class:on={cfg.large_files.resume} onclick={() => (cfg.large_files.resume = !cfg.large_files.resume)} aria-pressed={cfg.large_files.resume} aria-label={$t('settings_sync_lf_resume')}></button>
    </div>
    <p class="hint">{$t('settings_sync_lf_ram', { mib: String(peakMib) })}</p>
    <p class="warn">{$t('settings_sync_lf_chunk_warn')}</p>
  </div>
  {/if}

  {#if status?.joined}
    <div class="toggle-row">
      <div class="toggle-info">
        <span>{$t('settings_sync_enabled')}</span>
        <span class="hint">{$t('settings_sync_enabled_hint')}</span>
      </div>
      <button class="toggle" class:on={cfg.enabled} onclick={() => (cfg.enabled = !cfg.enabled)} aria-pressed={cfg.enabled} aria-label={$t('settings_sync_enabled')}></button>
    </div>
  {/if}

  <div class="row">
    <button class="btn btn-primary btn-sm" disabled={busy} onclick={save}>{$t('settings_backup_save')}</button>
    {#if status?.joined}
      <button class="btn btn-ghost btn-sm" disabled={busy || !isTauri || status.running} onclick={syncNow}>
        {status.running ? (progress ? `${progress.percent}%` : $t('settings_sync_running')) : $t('settings_sync_now')}
      </button>
      <button class="btn btn-ghost btn-sm" disabled={busy} onclick={() => (confirmLeave = true)}>{$t('settings_sync_leave')}</button>
    {:else}
      <button class="btn btn-ghost btn-sm" disabled={busy || !isTauri} onclick={probe}>{$t('settings_sync_probe')}</button>
    {/if}
    {#if saved}<span class="ok">✓</span>{/if}
  </div>

  {#if !status?.joined}
    <div class="field">
      <span class="field-label">{$t('settings_sync_passphrase')}</span>
      <div class="row">
        <input class="input" type="password" bind:value={passphrase} placeholder={$t('settings_sync_passphrase_placeholder')} autocomplete="new-password" />
        <button class="btn btn-primary btn-sm" disabled={busy || !isTauri || passphrase.length < 8} onclick={createVault}>{$t('settings_sync_create')}</button>
        <button class="btn btn-ghost btn-sm" disabled={busy || !isTauri || !passphrase} onclick={joinVault}>{$t('settings_sync_join')}</button>
      </div>
      <p class="hint">{$t('settings_sync_passphrase_hint')}</p>
    </div>
  {/if}

  {#if confirmLeave}
    <div class="confirm">
      <span>{$t('settings_sync_leave_confirm')}</span>
      <div class="row">
        <button class="btn btn-danger btn-sm" disabled={busy} onclick={leave}>{$t('settings_sync_leave')}</button>
        <button class="btn btn-ghost btn-sm" onclick={() => (confirmLeave = false)}>{$t('settings_backup_cancel')}</button>
      </div>
    </div>
  {/if}

  {#if status?.running && progress}
    <div class="progress-wrap">
      <div class="progress-bar"><div class="progress-fill" style="width: {progress.percent}%"></div></div>
      <span class="progress-label">
        {progressText}
        {#if progress.detail}<span class="progress-detail">{progress.detail}</span>{/if}
      </span>
    </div>
  {/if}

  {#if info}<p class="ok">{info}</p>{/if}
  {#if error}<p class="error">{error}</p>{/if}

  {#if status}
    <div class="table">
      <div class="trow"><span class="tlabel">{$t('settings_sync_status')}</span><span class="tvalue">{status.joined ? $t('settings_sync_status_joined') : $t('settings_sync_status_not_joined')}</span></div>
      {#if status.joined}
        <div class="trow"><span class="tlabel">{$t('settings_sync_vault_id')}</span><span class="tvalue mono">{status.vault_id?.slice(0, 12)}…</span></div>
        <div class="trow"><span class="tlabel">{$t('settings_sync_device_id')}</span><span class="tvalue mono">{status.device_id}</span></div>
        <div class="trow"><span class="tlabel">{$t('settings_sync_peers')}</span><span class="tvalue">{status.peers}</span></div>
        <button type="button" class="trow device-toggle" onclick={() => (devicesOpen = !devicesOpen)}>
          <span class="tlabel">{$t('settings_sync_storage_devices')}</span>
          <span class="tvalue">{status.storage_devices.length}</span>
        </button>
        {#if devicesOpen}
          {#each status.storage_devices as d (d.id)}
            <div class="trow device-card">
              <span class="tlabel">{d.name || $t('settings_sync_device_unnamed')}{d.own ? ` · ${$t('settings_sync_device_id')}` : ''}</span>
              <span class="tvalue mono">{d.id}</span>
            </div>
          {/each}
        {/if}
        <div class="trow"><span class="tlabel">{$t('settings_sync_last_applied')}</span><span class="tvalue">{status.last_applied ?? 0}</span></div>
        <div class="trow"><span class="tlabel">{$t('settings_sync_last_run')}</span><span class="tvalue">{status.last_run ? new Date(status.last_run).toLocaleString() : $t('settings_backup_never')}</span></div>
        {#if status.last_error}
          <div class="trow"><span class="tlabel">{$t('settings_sync_last_error')}</span><span class="tvalue error">{status.last_error}</span></div>
        {/if}
        {#if status.last_warning}
          <div class="trow"><span class="tlabel">{$t('settings_sync_last_warning')}</span><span class="tvalue warn">{status.last_warning}</span></div>
        {/if}
        {#if status.blobs_total !== null}
          <div class="trow" title={$t('settings_sync_gc_hint')}>
            <span class="tlabel">{$t('settings_sync_gc')}</span>
            <span class="tvalue">{$t('settings_sync_gc_value', { total: String(status.blobs_total), removed: String(status.blobs_removed_last_gc ?? 0), when: status.gc_last ? new Date(status.gc_last).toLocaleString() : '' })}</span>
          </div>
        {/if}
        {#if syncsFiles && status.lf_total !== null}
          <div class="trow" title={$t('settings_sync_gc_hint')}>
            <span class="tlabel">{$t('settings_sync_lf_gc')}</span>
            <span class="tvalue">{$t('settings_sync_gc_value', { total: String(status.lf_total), removed: String(status.lf_removed_last_gc ?? 0), when: status.gc_last ? new Date(status.gc_last).toLocaleString() : '' })}</span>
          </div>
        {/if}
      {/if}
    </div>

    {#if status.conflicts.length > 0}
      <div class="field">
        <span class="field-label">{$t('settings_sync_conflicts')}</span>
        <p class="hint">{$t('settings_sync_conflicts_hint')}</p>
        <ul class="conflicts">
          {#each status.conflicts as c (c.note_id)}
            <!-- The owner of notes says where a note opens; without it the title alone. -->
            {@const href = directory.get('note')?.href?.(c.note_id, 'conflict')}
            <li>{#if href}<a {href}>{c.title}</a>{:else}{c.title}{/if}</li>
          {/each}
        </ul>
      </div>
    {/if}

    {#if status.profile_conflicts.length > 0}
      <div class="field">
        <span class="field-label">{$t('settings_sync_profile_conflicts')}</span>
        <p class="hint">{$t('profile_sync_diverged_hint')}</p>
        <ul class="conflicts">
          {#each status.profile_conflicts as c (c.note_id)}
            <li>{c.title}</li>
          {/each}
        </ul>
      </div>
    {/if}

    {#if status.joined}
      <details class="passphrase">
        <summary>{$t('settings_sync_change_passphrase')}</summary>
        <p class="hint">{$t('settings_sync_change_passphrase_hint')}</p>
        <div class="row">
          <input class="input" type="password" bind:value={oldPassphrase} placeholder={$t('settings_sync_passphrase_old')} autocomplete="off" />
          <input class="input" type="password" bind:value={newPassphrase} placeholder={$t('settings_sync_passphrase_new')} autocomplete="new-password" />
          <button class="btn btn-ghost btn-sm" disabled={busy || !oldPassphrase || newPassphrase.length < 8} onclick={changePassphrase}>{$t('settings_lock_change')}</button>
        </div>
      </details>
    {/if}
  {/if}
</div>

<style>
  .sync { display: flex; flex-direction: column; gap: var(--sp-4); }
  .row { display: flex; gap: var(--sp-2); align-items: center; }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: var(--sp-3); }
  .field { display: flex; flex-direction: column; gap: var(--sp-2); min-width: 0; }
  .field.wide { grid-column: 1 / -1; }
  .field-label { font-size: var(--fs-sm); color: var(--text); font-weight: var(--fw-semibold); }
  /* In a row the input shares the width; in a column field (`label.field`)
     a flex-basis of 0 would be its height, so only rows stretch it. */
  .row > .input { flex: 1; }
  .input {
    width: 100%; min-width: 0; height: var(--control-h-lg); flex-shrink: 0;
    background: var(--surface-3); border: 1px solid var(--border);
    border-radius: var(--radius-field); padding: 0 var(--sp-3);
    font-size: 0.82rem; color: var(--text-body);
  }
  .row > .input.interval { flex: 0 0 110px; }
  .presets { display: flex; flex-wrap: wrap; gap: var(--sp-2); align-items: center; }
  .chip {
    font-size: var(--fs-sm); padding: 0.15rem 0.55rem; border-radius: var(--radius-field);
    border: 1px solid var(--border); background: var(--surface-3); color: var(--text-dim); cursor: pointer;
  }
  .chip:hover { color: var(--text); border-color: var(--border-2); }
  .chip.on { color: var(--accent-text); background: var(--accent-bg); border-color: var(--accent-border); }
  .warn { font-size: var(--fs-sm); color: var(--warn-text); margin: 0; }
  .input:focus { border-color: var(--accent-border); box-shadow: 0 0 0 3px var(--accent-bg); outline: none; }
  .icon { width: var(--control-h-lg); height: var(--control-h-lg); justify-content: center; padding: 0; flex-shrink: 0; }
  .hint { font-size: var(--fs-sm); color: var(--text-dim); margin: 0; }
  .ok { font-size: var(--fs-sm); color: var(--success-text); margin: 0; }
  .error { font-size: var(--fs-sm); color: var(--danger-text); margin: 0; }
  .tvalue.warn { color: var(--warn-text); }
  .progress-wrap { display: flex; flex-direction: column; gap: 0.3rem; }
  .progress-bar { height: 6px; background: var(--surface-3); border-radius: 999px; overflow: hidden; }
  .progress-fill { height: 100%; background: var(--accent); border-radius: 999px; transition: width 0.2s ease; }
  .progress-label { font-size: var(--fs-sm); color: var(--text-body); }
  .progress-detail { display: block; font-family: var(--font-mono); color: var(--text-dim); word-break: break-all; }
  .toggle-row { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-4); }
  .toggle-info { display: flex; flex-direction: column; gap: 0.2rem; font-size: var(--fs-base); }
  .confirm { display: flex; flex-direction: column; gap: var(--sp-2); padding: var(--sp-3); border: 1px solid var(--danger-border); border-radius: var(--radius); background: var(--danger-bg); font-size: var(--fs-sm); }
  .table { display: flex; flex-direction: column; gap: 0.35rem; }
  .trow { display: flex; align-items: baseline; gap: var(--sp-2); }
  .tlabel { font-size: 0.82rem; color: var(--text-faint); min-width: 120px; flex-shrink: 0; }
  .device-toggle { width: 100%; background: none; border: 0; padding: 0; cursor: pointer; text-align: left; color: inherit; }
  .device-card { flex-direction: column; align-items: stretch; gap: 2px; padding-left: var(--sp-3); }
  .device-card .tlabel { min-width: 0; word-break: break-word; }
  .tvalue { font-size: var(--fs-sm); color: var(--text-body); word-break: break-all; }
  .conflicts { margin: 0; padding-left: 1.2rem; font-size: var(--fs-sm); }
  .conflicts a { color: var(--warn-text); }
  .passphrase summary { font-size: var(--fs-sm); color: var(--text-dim); cursor: pointer; }
  .passphrase .row { margin-top: var(--sp-2); }
</style>
