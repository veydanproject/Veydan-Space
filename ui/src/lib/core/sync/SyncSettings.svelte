<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { t, locale } from '$lib/core/i18n';
  import { api, LARGE_FILE_LIMITS, largeFilePeakMib } from '$lib/core/api';
  import type { SyncConfig, SyncStatus } from '$lib/core/api';
  import { syncStore } from '$lib/core/store/sync.svelte';
  import { formatBytes, formatDateTime, formatError } from '$lib/core/utils';
  import { joinErrorKey, storageErrorKey } from '$lib/core/join-error';
  import CustomSelect from '$lib/core/ui/CustomSelect.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { directory } from '$lib/core/directory';
  import { registry } from '$lib/core/registry';
  import { goto } from '$app/navigation';
  import { ask } from '$lib/core/ui/confirm.svelte';
  import { storageOf } from '$lib/core/sync/storage';
  import { CLOUD_BACKEND, asksForProfileFiles, filesLimit, fullestPool, loadCloud, usageTone, type CloudView } from '$lib/core/sync/cloud';

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

  /** Veydan Cloud as the module cloud tells it; nothing where the module is off. */
  let cloud = $state<CloudView>({ choice: 'none', plan: null, usage: null });
  const onCloud = $derived(cfg.backend === CLOUD_BACKEND);
  const fullest = $derived(fullestPool(cloud.usage));
  const tone = $derived(fullest ? usageTone(fullest.level) : 'none');
  const usageLine = $derived.by(() => {
    if (!fullest || tone === 'none') return '';
    const args = { used: formatBytes(fullest.used), limit: formatBytes(fullest.limit), percent: String(fullest.percent) };
    if (tone === 'full') return $t('settings_sync_cloud_full', args);
    if (tone === 'warn') return $t('settings_sync_cloud_almost_full', args);
    return $t('settings_sync_cloud_used', args);
  });

  const backendOptions = $derived([
    { value: 'folder', label: $t('settings_sync_backend_folder') },
    { value: 's3', label: $t('settings_sync_backend_s3') },
    { value: 'webdav', label: $t('settings_sync_backend_webdav') },
    // Offered with an account; kept in the list while it is the backend in use.
    ...(cloud.choice === 'option' || onCloud ? [{ value: CLOUD_BACKEND, label: $t('settings_sync_backend_cloud') }] : []),
  ]);

  function chooseBackend(value: string | null | undefined) {
    const next = value ?? 'folder';
    // The free plan: the files of profiles stay off until the user says yes (plan 4.7).
    if (next !== cfg.backend && asksForProfileFiles(next, cloud.plan)) cfg.profile_files = false;
    cfg.backend = next;
  }

  async function toggleProfileFiles() {
    if (!cfg.profile_files && asksForProfileFiles(cfg.backend, cloud.plan)) {
      const limit = filesLimit(cloud.usage);
      const yes = await ask({
        title: $t('settings_sync_cloud_profiles_title'),
        message: $t('settings_sync_cloud_profiles_ask', {
          size: formatBytes(cloud.usage?.profile_files_estimate ?? 0),
          limit: limit ? formatBytes(limit) : '—',
        }),
        confirmLabel: $t('settings_sync_cloud_profiles_yes'),
        variant: 'primary',
      });
      if (!yes) return;
    }
    cfg.profile_files = !cfg.profile_files;
  }

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

  /** The last error of the cycle, in words where the storage itself is refused. */
  function lastErrorText(message: string): string {
    const key = storageErrorKey(message);
    return key ? $t(key) : message;
  }

  async function refreshStatus() {
    try {
      status = await api.sync.status();
    } catch {}
  }

  /** The config as it is saved: the storage a joined device syncs through. */
  let stored: SyncConfig | null = null;

  async function loadConfig() {
    cfg = await api.sync.getConfig();
    stored = $state.snapshot(cfg);
  }

  async function run(task: () => Promise<unknown>) {
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

  function showSaved() {
    saved = true;
    setTimeout(() => (saved = false), 2000);
  }

  /** Saves the form. False where the device still syncs as before, or has only now got its vault: no sync is to follow. */
  async function saveConfig(): Promise<boolean> {
    if (status?.joined && stored && storageOf(cfg) !== storageOf(stored)) return changeStorage();
    await api.sync.setConfig($state.snapshot(cfg));
    await loadConfig();
    showSaved();
    return true;
  }

  /**
   * The form of a joined device names another storage. The device is bound
   * to the vault of the old one: saved alone, the new storage would leave
   * sync looking there for a vault that is not there. So the device leaves
   * its vault and takes the one of the new storage, created where there is
   * none. Veydan Cloud asks no passphrase and is finished here; another
   * storage is finished in the form, which asks for one.
   */
  async function changeStorage(): Promise<boolean> {
    const yes = await ask({
      title: $t('settings_sync_move_title'),
      message: $t('settings_sync_move_ask'),
      confirmLabel: $t('settings_sync_move_yes'),
      variant: 'primary',
    });
    if (!yes) return false;
    const found = await api.sync.changeStorage($state.snapshot(cfg));
    if (found === 'foreign') {
      error = $t('settings_sync_probe_foreign');
      return false;
    }
    await loadConfig();
    if (found === 'same') {
      showSaved();
      return true;
    }
    if (!onCloud) {
      info = $t(`settings_sync_probe_${found}`);
      return false;
    }
    status = found === 'vault' ? await api.sync.joinVault('') : await api.sync.createVault('');
    await loadConfig();
    info = $t(found === 'vault' ? 'settings_sync_joined' : 'settings_sync_created');
    return false;
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
      await loadConfig();
      passphrase = '';
      info = $t('settings_sync_created');
    });

  const joinVault = () =>
    run(async () => {
      await saveConfig();
      status = await api.sync.joinVault(passphrase);
      await loadConfig();
      passphrase = '';
      info = $t('settings_sync_joined');
    });

  const syncNow = () =>
    run(async () => {
      if (!(await saveConfig())) return;
      status = await api.sync.runNow();
      info = $t('settings_sync_done');
    });

  const leave = () =>
    run(async () => {
      confirmLeave = false;
      status = await api.sync.leave();
      await loadConfig();
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
      await loadConfig();
      intervalUnit = unitFor(cfg.interval_sec);
    } catch {}
    void loadCloud().then((view) => (cloud = view));
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

<!-- Joined: the status first (its actions, its banners, its facts), then the conflicts. Then the storage
     (a new device: the passphrase and the two ways in, at its foot), the interval, this device, the large
     files, the passphrase's change; the save at the foot of the page. -->
<div class="sync">
  {#if status?.joined}
    <div class="card status">
      <div class="status-head">
        <span class="state"><span class="state-title">{$t('settings_sync_status')}</span><span class="badge badge-ok">{$t('settings_sync_status_joined')}</span></span>
        <span class="acts">
          <button type="button" class="btn btn-ghost" disabled={busy || !isTauri || status.running} onclick={syncNow}>
            <Icon name="refresh-cw" size={18} />
            {status.running ? (progress ? `${progress.percent}%` : $t('settings_sync_running')) : $t('settings_sync_now')}
          </button>
          <button type="button" class="btn btn-ghost leave" disabled={busy} onclick={() => (confirmLeave = true)}>{$t('settings_sync_leave')}</button>
        </span>
      </div>

      {#if status.running && progress}
        <div class="progress-wrap" role="status">
          <div class="progress-bar"><div class="progress-fill" style="width: {progress.percent}%"></div></div>
          <span class="progress-label">
            {progressText}
            {#if progress.detail}<span class="progress-detail">{progress.detail}</span>{/if}
          </span>
        </div>
      {/if}

      {#if confirmLeave}
        <div class="banner danger" role="alert">
          <Icon name="alert-triangle" size={18} />
          <span class="banner-words">
            <span>{$t('settings_sync_leave_confirm')}</span>
            <span class="btns">
              <button type="button" class="btn btn-danger btn-sm" disabled={busy} onclick={leave}>{$t('settings_sync_leave')}</button>
              <button type="button" class="btn btn-ghost btn-sm" onclick={() => (confirmLeave = false)}>{$t('settings_backup_cancel')}</button>
            </span>
          </span>
        </div>
      {/if}
      {#if status.last_error}
        <div class="banner danger" role="alert"><Icon name="alert-triangle" size={18} /><span><b>{$t('settings_sync_last_error')}</b> · {lastErrorText(status.last_error)}</span></div>
      {/if}
      {#if status.last_warning}
        <div class="banner warn"><Icon name="alert-triangle" size={18} /><span><b>{$t('settings_sync_last_warning')}</b> · {status.last_warning}</span></div>
      {/if}
      {#if status.first_read_pending}
        <div class="banner warn"><Icon name="info" size={18} /><span>{$t('settings_sync_first_read')}</span></div>
      {/if}

      <div class="facts">
        <div class="fact"><span class="fact-key">{$t('settings_sync_vault_id')}</span><span class="fact-value mono">{status.vault_id?.slice(0, 12)}…</span></div>
        <div class="fact"><span class="fact-key">{$t('settings_sync_device_id')}</span><span class="fact-value mono">{status.device_id}</span></div>
        <div class="fact"><span class="fact-key">{$t('settings_sync_peers')}</span><span class="fact-value">{status.peers}</span></div>
        <button type="button" class="fact device-toggle" aria-expanded={devicesOpen} onclick={() => (devicesOpen = !devicesOpen)}>
          <span class="fact-key">{$t('settings_sync_storage_devices')}</span>
          <span class="fact-value">{status.storage_devices.length}</span>
          <Icon name={devicesOpen ? 'chevron-down' : 'chevron-right'} size={18} />
        </button>
        {#if devicesOpen}
          <div class="devices">
            {#each status.storage_devices as d (d.id)}
              <div class="device">
                <span class="device-name">{d.name || $t('settings_sync_device_unnamed')}{d.own ? ` · ${$t('settings_sync_device_id')}` : ''}</span>
                <span class="mono device-id">{d.id}</span>
              </div>
            {/each}
          </div>
        {/if}
        <div class="fact"><span class="fact-key">{$t('settings_sync_last_applied')}</span><span class="fact-value">{status.last_applied ?? 0}</span></div>
        <div class="fact"><span class="fact-key">{$t('settings_sync_last_run')}</span><span class="fact-value">{status.last_run ? formatDateTime(status.last_run, $locale) : $t('settings_backup_never')}</span></div>
        {#if status.blobs_total !== null}
          <div class="fact" title={$t('settings_sync_gc_hint')}>
            <span class="fact-key">{$t('settings_sync_gc')}</span>
            <span class="fact-value">{$t('settings_sync_gc_value', { total: String(status.blobs_total), removed: String(status.blobs_removed_last_gc ?? 0), when: status.gc_last ? formatDateTime(status.gc_last, $locale) : '' })}</span>
          </div>
        {/if}
        {#if syncsFiles && status.lf_total !== null}
          <div class="fact" title={$t('settings_sync_gc_hint')}>
            <span class="fact-key">{$t('settings_sync_lf_gc')}</span>
            <span class="fact-value">{$t('settings_sync_gc_value', { total: String(status.lf_total), removed: String(status.lf_removed_last_gc ?? 0), when: status.gc_last ? formatDateTime(status.gc_last, $locale) : '' })}</span>
          </div>
        {/if}
      </div>
    </div>

  {/if}

  <!-- The conflicts whenever there is a status, joined or not. -->
  {#if status}
    {#if status.conflicts.length > 0}
      <section class="group">
        <div class="caption">{$t('settings_sync_conflicts')}</div>
        <p class="hint">{$t('settings_sync_conflicts_hint')}</p>
        <div class="card list">
          {#each status.conflicts as c (c.note_id)}
            <!-- The owner of notes says where a note opens; without it the title alone. -->
            {@const href = directory.get('note')?.href?.(c.note_id, 'conflict')}
            {#if href}
              <a class="item conflict" {href}><Icon name="alert-triangle" size={18} /><span class="grow">{c.title}</span><Icon name="chevron-right" size={18} /></a>
            {:else}
              <div class="item conflict"><Icon name="alert-triangle" size={18} /><span class="grow">{c.title}</span></div>
            {/if}
          {/each}
        </div>
      </section>
    {/if}

    {#if status.profile_conflicts.length > 0}
      <section class="group">
        <div class="caption">{$t('settings_sync_profile_conflicts')}</div>
        <p class="hint">{$t('profile_sync_diverged_hint')}</p>
        <div class="card list">
          {#each status.profile_conflicts as c (c.note_id)}
            <div class="item"><span class="grow">{c.title}</span></div>
          {/each}
        </div>
      </section>
    {/if}
  {/if}

  {#if status && !status.joined}
    <!-- Not joined: the status alone, as the joined device's head says it. -->
    <div class="card status">
      <div class="status-head">
        <span class="state"><span class="state-title">{$t('settings_sync_status')}</span><span class="badge">{$t('settings_sync_status_not_joined')}</span></span>
      </div>
    </div>
  {/if}

  <section class="group">
    <div class="caption">{$t('settings_sync_backend')}</div>
    <div class="card block">
      <div class="pad">
        <div class="field">
          <!-- The block's caption names the choice: no label of its own repeating it. -->
          <CustomSelect options={backendOptions} value={cfg.backend} onchange={chooseBackend} />
          {#if cloud.choice === 'activate' && !onCloud}
            <div class="row">
              <span class="hint">{$t('settings_sync_cloud_activate_hint')}</span>
              <button type="button" class="btn btn-ghost btn-sm" onclick={() => goto('/cloud')}>{$t('settings_sync_cloud_activate')}</button>
            </div>
          {/if}
        </div>

        {#if cfg.backend === 'folder'}
          <div class="field">
            <span class="field-label">{$t('settings_sync_folder')}</span>
            <div class="row">
              <input class="input mono" type="text" bind:value={cfg.folder_path} placeholder={$t('settings_sync_folder_placeholder')} aria-label={$t('settings_sync_folder')} readonly={!isTauri} />
              {#if isTauri}
                <button type="button" class="btn btn-ghost browse" onclick={browseFolder}>
                  <Icon name="folder-open" size={18} />
                  {$t('settings_backup_browse')}
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
        {:else if onCloud}
          <p class="hint">{$t('settings_sync_cloud_hint')}</p>
          {#if usageLine}
            <p class={tone === 'quiet' ? 'hint' : 'warn'}>{usageLine}</p>
          {/if}
          {#if status?.joined}
            <p class="hint">{$t('settings_sync_cloud_move_hint')}</p>
          {/if}
        {:else}
          <div class="grid">
            <label class="field wide"><span class="field-label">{$t('settings_sync_webdav_url')}</span><input class="input" type="text" bind:value={cfg.webdav.url} placeholder="https://webdav.example.com/veydan" /></label>
            <label class="field"><span class="field-label">{$t('settings_sync_webdav_username')}</span><input class="input" type="text" bind:value={cfg.webdav.username} autocomplete="off" /></label>
            <label class="field"><span class="field-label">{$t('settings_sync_webdav_password')}</span><input class="input" type="password" bind:value={cfg.webdav.password} placeholder={cfg.webdav.has_password && cfg.webdav.password == null ? $t('secret_stored_placeholder') : ''} autocomplete="off" /></label>
          </div>
        {/if}
      </div>

      {#if cfg.backend === 's3'}
        {@render switchRow(cfg.s3.path_style, $t('settings_sync_s3_path_style'), $t('settings_sync_s3_path_style_hint'), () => (cfg.s3.path_style = !cfg.s3.path_style))}
      {/if}

      {#if !status?.joined && onCloud}
        <!-- The passphrase of Veydan Cloud comes from the identity: the backend derives it. -->
        <div class="pad join">
          <div class="btns">
            <button type="button" class="btn btn-primary" disabled={busy || !isTauri} onclick={createVault}>{$t('settings_sync_create')}</button>
            <button type="button" class="btn btn-ghost" disabled={busy || !isTauri} onclick={joinVault}>{$t('settings_sync_join')}</button>
          </div>
          <p class="hint">{$t('settings_sync_cloud_passphrase_hint')}</p>
        </div>
      {:else if !status?.joined}
        <div class="pad join">
          <div class="field">
            <span class="field-label">{$t('settings_sync_passphrase')}</span>
            <div class="row wrap">
              <input class="input" type="password" bind:value={passphrase} placeholder={$t('settings_sync_passphrase_placeholder')} aria-label={$t('settings_sync_passphrase')} autocomplete="new-password" />
              <button type="button" class="btn btn-primary" disabled={busy || !isTauri || passphrase.length < 8} onclick={createVault}>{$t('settings_sync_create')}</button>
              <button type="button" class="btn btn-ghost" disabled={busy || !isTauri || !passphrase} onclick={joinVault}>{$t('settings_sync_join')}</button>
            </div>
            <p class="hint">{$t('settings_sync_passphrase_hint')}</p>
          </div>
        </div>
      {/if}
    </div>
  </section>

  <section class="group">
    <div class="caption">{$t('settings_sync_interval')}</div>
    <div class="card block">
      <div class="pad">
        <div class="row">
          <input class="input interval" type="number" min="1" max={intervalUnit === 'min' ? MAX_INTERVAL_SEC / 60 : MAX_INTERVAL_SEC} value={intervalValue} aria-label={$t('settings_sync_interval')} oninput={(e) => setIntervalValue(Number((e.currentTarget as HTMLInputElement).value))} />
          <div class="unit"><CustomSelect options={unitOptions} value={intervalUnit} onchange={(v) => setIntervalUnit(v === 'sec' ? 'sec' : 'min')} /></div>
        </div>
        <div class="presets">
          <span class="hint">{$t('settings_sync_interval_presets')}</span>
          {#each presets as sec (sec)}
            <button type="button" class="chip" class:on={cfg.interval_sec === sec} onclick={() => applyPreset(sec)}>{presetLabel(sec)}</button>
          {/each}
        </div>
        {#if remoteFastWarn}
          <div class="banner warn"><Icon name="alert-triangle" size={18} /><span>{$t('settings_sync_interval_remote_warn')}</span></div>
        {/if}
      </div>
    </div>
  </section>

  <div class="card block">
    <div class="pad">
      <label class="field">
        <span class="field-label">{$t('settings_sync_device_name')}</span>
        <input class="input" type="text" bind:value={cfg.device_name} maxlength="64" />
        <span class="hint">{hasProfiles ? $t('settings_sync_device_name_profiles_hint') : $t('settings_sync_device_name_hint')}</span>
      </label>
    </div>
    {#if hasProfiles}
      {@render switchRow(cfg.profile_files, $t('settings_sync_profile_files'), $t('settings_sync_profile_files_hint'), toggleProfileFiles)}
    {/if}
    {#if status?.joined}
      {@render switchRow(cfg.enabled, $t('settings_sync_enabled'), $t('settings_sync_enabled_hint'), () => (cfg.enabled = !cfg.enabled))}
    {/if}
  </div>

  {#if syncsFiles}
    <section class="group">
      <div class="caption">{$t('settings_sync_lf_section')}</div>
      <div class="card block">
        <div class="pad">
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
        </div>
        {@render switchRow(cfg.large_files.resume, $t('settings_sync_lf_resume'), $t('settings_sync_lf_resume_hint'), () => (cfg.large_files.resume = !cfg.large_files.resume))}
        <div class="pad notes">
          <p class="note"><Icon name="info" size={18} /><span>{$t('settings_sync_lf_ram', { mib: String(peakMib) })}</span></p>
          <p class="note warn"><Icon name="alert-triangle" size={18} /><span>{$t('settings_sync_lf_chunk_warn')}</span></p>
        </div>
      </div>
    </section>
  {/if}

  {#if status?.joined && !onCloud}
    <details class="card passphrase">
      <summary class="item">
        <span class="grow switch-title">{$t('settings_sync_change_passphrase')}</span>
        <Icon name="chevron-down" size={18} />
      </summary>
      <div class="pad">
        <p class="hint">{$t('settings_sync_change_passphrase_hint')}</p>
        <div class="row wrap">
          <input class="input" type="password" bind:value={oldPassphrase} placeholder={$t('settings_sync_passphrase_old')} aria-label={$t('settings_sync_passphrase_old')} autocomplete="off" />
          <input class="input" type="password" bind:value={newPassphrase} placeholder={$t('settings_sync_passphrase_new')} aria-label={$t('settings_sync_passphrase_new')} autocomplete="new-password" />
          <button type="button" class="btn btn-ghost" disabled={busy || !oldPassphrase || newPassphrase.length < 8} onclick={changePassphrase}>{$t('settings_lock_change')}</button>
        </div>
      </div>
    </details>
  {/if}

  {#if info}<div class="banner ok" role="status"><span>{info}</span></div>{/if}
  {#if error}<div class="banner danger" role="alert"><Icon name="alert-triangle" size={18} /><span>{error}</span></div>{/if}

  <div class="foot">
    {#if saved}<span class="saved" role="status">{$t('settings_sync_saved')}</span>{/if}
    {#if !status?.joined}
      <button type="button" class="btn btn-ghost" disabled={busy || !isTauri} onclick={probe}>{$t('settings_sync_probe')}</button>
    {/if}
    <button type="button" class="btn btn-primary" disabled={busy} onclick={save}>{$t('settings_backup_save')}</button>
  </div>
</div>

{#snippet switchRow(on: boolean, title: string, hint: string, flip: () => void)}
  <button type="button" class="switch-row" role="switch" aria-checked={on} onclick={flip}>
    <span class="words">
      <span class="switch-title">{title}</span>
      <span class="hint">{hint}</span>
    </span>
    <span class="toggle" class:on aria-hidden="true"></span>
  </button>
{/snippet}

<style>
  .sync { display: flex; flex-direction: column; gap: var(--sp-4); min-width: 0; container-type: inline-size; }
  .row { display: flex; gap: var(--sp-2); align-items: center; }
  .row.wrap { flex-wrap: wrap; }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: var(--sp-4); }
  .field { display: flex; flex-direction: column; gap: var(--sp-2); min-width: 0; }
  .field.wide { grid-column: 1 / -1; }
  .field-label { font-size: var(--fs-sm); color: var(--text); font-weight: var(--fw-semibold); }
  /* In a row the input shares the width; in a column field (`label.field`)
     a flex-basis of 0 would be its height, so only rows stretch it. */
  .row > .input { flex: 1; min-width: 180px; }
  .input {
    width: 100%; min-width: 0; height: var(--control-h-lg); flex-shrink: 0;
    background: var(--surface-3); border: 1px solid var(--border);
    border-radius: var(--radius-field); padding: 0 var(--sp-3);
    font-size: var(--fs-sm); color: var(--text-body);
  }
  .row > .input.interval { flex: 0 0 110px; min-width: 0; }
  .unit { flex: 0 0 180px; }
  .browse { height: var(--control-h-lg); flex-shrink: 0; }
  .presets { display: flex; flex-wrap: wrap; gap: var(--sp-2); align-items: center; }
  .chip {
    font-size: var(--fs-sm); height: var(--control-h-sm); padding: 0 var(--sp-3); border-radius: var(--radius-pill);
    border: 1px solid var(--border); background: var(--surface-3); color: var(--text-dim); cursor: pointer;
  }
  .chip:hover { color: var(--text); border-color: var(--border-2); }
  .chip.on { color: var(--accent-text); background: var(--accent-bg); border-color: var(--accent-border); }
  .warn { font-size: var(--fs-sm); color: var(--warn-text); margin: 0; }
  .input:focus { border-color: var(--accent-border); box-shadow: 0 0 0 3px var(--accent-bg); outline: none; }
  .hint { font-size: var(--fs-sm); line-height: 1.45; color: var(--text-dim); margin: 0; }

  .group { display: flex; flex-direction: column; gap: var(--sp-2); }
  .caption { padding: 0 var(--sp-1); font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text-2); }
  .group > .hint { padding: 0 var(--sp-1); }
  .card.block { padding: var(--sp-1); }
  .pad { padding: var(--sp-3); display: flex; flex-direction: column; gap: var(--sp-4); }
  .join { border-top: 1px solid var(--divider); }
  .btns { display: flex; gap: var(--sp-2); flex-wrap: wrap; }
  .notes { gap: var(--sp-2); border-top: 1px solid var(--divider); }
  .note { display: flex; gap: var(--sp-3); align-items: flex-start; margin: 0; font-size: var(--fs-sm); color: var(--text-2); }
  .note :global(svg) { flex-shrink: 0; color: var(--text-dim); }
  .note.warn, .note.warn :global(svg) { color: var(--warn-text); }

  .switch-row {
    position: relative; display: flex; align-items: center; gap: var(--sp-3); width: 100%; min-height: 64px; padding: var(--sp-2) var(--sp-4);
    border: 0; border-radius: 0; background: none; color: var(--text); font: inherit; text-align: left; cursor: pointer;
    transition: background var(--dur-fast);
  }
  /* The hover of a row: square between lines, the first and the last row take the card's corners (its radius less its padding). */
  .switch-row:first-child { border-top-left-radius: calc(var(--radius-card) - var(--sp-1)); border-top-right-radius: calc(var(--radius-card) - var(--sp-1)); }
  .switch-row:last-child { border-bottom-left-radius: calc(var(--radius-card) - var(--sp-1)); border-bottom-right-radius: calc(var(--radius-card) - var(--sp-1)); }
  /* A straight line over each switch: a shadow would follow the rounded corners. */
  .switch-row::before { content: ''; position: absolute; top: 0; left: 0; right: 0; height: 1px; background: var(--divider); }
  .switch-row:hover { background: var(--surface-row-hover); }
  .switch-row .toggle { pointer-events: none; }
  .words { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .switch-title { font-size: var(--fs-body); font-weight: var(--fw-semibold); color: var(--text); }

  .card.status { padding: var(--sp-4); display: flex; flex-direction: column; gap: var(--sp-3); }
  .status-head { display: flex; align-items: center; gap: var(--sp-3); flex-wrap: wrap; }
  .state { flex: 1; display: flex; align-items: center; gap: var(--sp-3); }
  .state-title { font-size: var(--fs-md); font-weight: var(--fw-semibold); }
  .acts { display: flex; gap: var(--sp-2); }
  .acts .btn { display: inline-flex; align-items: center; gap: var(--sp-2); }
  .leave { color: var(--danger-text); }
  .progress-wrap { display: flex; flex-direction: column; gap: 0.3rem; }
  .progress-bar { height: 6px; background: var(--surface-3); border-radius: var(--radius-pill); overflow: hidden; }
  .progress-fill { height: 100%; background: var(--accent); border-radius: var(--radius-pill); transition: width 0.2s ease; }
  .progress-label { font-size: var(--fs-sm); color: var(--text-body); }
  .progress-detail { display: block; font-family: var(--font-mono); color: var(--text-dim); word-break: break-all; }
  .banner {
    display: flex; align-items: flex-start; gap: var(--sp-3); padding: var(--sp-3) var(--sp-4);
    border-radius: var(--radius-md); font-size: var(--fs-sm); line-height: 1.5; overflow-wrap: anywhere;
  }
  .banner :global(svg) { flex-shrink: 0; margin-top: 1px; }
  .banner.danger { background: var(--danger-bg); color: var(--text); }
  .banner.danger :global(svg) { color: var(--danger-text); }
  .banner.warn { background: var(--warn-bg, var(--surface-2)); color: var(--warn-text); }
  .banner.ok { background: var(--success-bg); color: var(--success-text); }
  .banner-words { display: flex; flex-direction: column; gap: var(--sp-2); }
  .facts { margin: 0 calc(-1 * var(--sp-4)); padding: var(--sp-1); border-top: 1px solid var(--divider); display: flex; flex-direction: column; }
  .fact { position: relative; display: flex; align-items: baseline; gap: var(--sp-4); min-height: 40px; padding: 10px var(--sp-4); }
  .fact-key { flex: 0 0 220px; font-size: var(--fs-sm); color: var(--text-dim); }
  .fact-value { flex: 1; min-width: 0; font-size: var(--fs-sm); color: var(--text-body); word-break: break-word; }
  .device-toggle { width: 100%; align-items: center; background: none; border: 0; cursor: pointer; text-align: left; color: inherit; font: inherit; border-radius: var(--radius-md); }
  .device-toggle:hover { background: var(--surface-row-hover); }
  .device-toggle .fact-value { flex: 0 0 auto; }
  .devices { margin: 0 var(--sp-4) var(--sp-1) calc(220px + var(--sp-4) * 2); padding-left: var(--sp-3); border-left: 2px solid var(--divider); display: flex; flex-direction: column; gap: var(--sp-2); }
  .device { display: flex; flex-direction: column; gap: 2px; }
  .device-name { font-size: var(--fs-sm); color: var(--text); word-break: break-word; }
  .device-id { font-size: var(--fs-xs); color: var(--text-dim); word-break: break-all; }

  .card.list { padding: var(--sp-1); display: flex; flex-direction: column; }
  .item { position: relative; display: flex; align-items: center; gap: var(--sp-3); min-height: 48px; padding: var(--sp-2) var(--sp-4); color: var(--text); text-decoration: none; font-size: var(--fs-base); border-radius: var(--radius-md); }
  .item + .item::before { content: ''; position: absolute; top: 0; left: 0; right: 0; height: 1px; background: var(--divider); }
  .conflict :global(svg:first-child) { color: var(--warn-text); flex-shrink: 0; }
  a.item:hover { background: var(--surface-row-hover); }
  .grow { flex: 1; min-width: 0; }

  .card.passphrase { padding: var(--sp-1); }
  .passphrase summary { list-style: none; cursor: pointer; min-height: 56px; }
  .passphrase summary::-webkit-details-marker { display: none; }
  .passphrase[open] summary :global(svg) { transform: rotate(180deg); }
  .passphrase .pad { border-top: 1px solid var(--divider); }

  .foot { display: flex; justify-content: flex-end; align-items: center; flex-wrap: wrap; gap: var(--sp-2); }
  .saved { margin-right: var(--sp-1); font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--success-text); }

  /* A narrow window: a fact's name over its value, the devices under it. */
  @container (max-width: 600px) {
    .fact { flex-direction: column; align-items: flex-start; gap: 2px; }
    .fact-key { flex-basis: auto; }
    .device-toggle { flex-direction: row; align-items: center; }
    .devices { margin-left: var(--sp-4); }
  }
</style>
