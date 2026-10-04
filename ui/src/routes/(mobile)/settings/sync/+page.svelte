<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { ask } from '$lib/core/ui/confirm.svelte';
  import { onMount } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { api, formatError, onSyncProgress, onSyncStatus, syncProgressText, type SyncConfig, type SyncProbe, type SyncProgress, type SyncStatus } from '$lib/core/mobile/api';
  import { t, locale } from '$lib/core/mobile/i18n';
  import { joinErrorKey } from '$lib/core/join-error';
  import { LARGE_FILE_LIMITS, largeFilePeakMib } from '$lib/core/api';
  import { registry } from '$lib/core/registry';
  import { directory } from '$lib/core/directory';

  let cfg = $state<SyncConfig | null>(null);
  /** The modules' parts of this form (the notes' attachment policy), saved with it. */
  /** Large-file transfer applies where a module syncs files (notes' attachments, profile files). */
  const syncsFiles = registry.modules.some((m) => m.syncsFiles);
  const blocks = registry.settings().filter((sec) => sec.into === 'sync' && (sec.visible?.() ?? true));
  let blockRefs = $state<Record<string, unknown>>({});

  const clampNum = (raw: string, [lo, hi]: readonly [number, number]) => Math.min(hi, Math.max(lo, Math.floor(Number(raw) || lo)));
  let status = $state<SyncStatus | null>(null);
  let devicesOpen = $state(false);
  let probe = $state<SyncProbe | null>(null);
  let progress = $state<SyncProgress | null>(null);
  let passphrase = $state('');
  let busy = $state(false);
  let error = $state('');
  let toast = $state('');
  let toastTimer: ReturnType<typeof setTimeout>;
  let saveState = $state<'idle' | 'saving' | 'saved'>('idle');
  let saveResetTimer: ReturnType<typeof setTimeout>;
  let syncBtnState = $state<'idle' | 'syncing' | 'done'>('idle');
  let syncResetTimer: ReturnType<typeof setTimeout>;

  const running = $derived(!!status?.running);
  const progressLine = $derived(progress ? syncProgressText($t, progress) : '');
  const syncVisual = $derived(
    syncBtnState === 'done' ? 'done'
      : running || syncBtnState === 'syncing' ? 'syncing'
        : 'idle',
  );

  const intervals = [30, 60, 300, 900, 3600];

  function showToast(msg: string) {
    clearTimeout(toastTimer);
    toast = msg;
    toastTimer = setTimeout(() => (toast = ''), 2000);
  }

  function intervalLabel(sec: number): string {
    return sec < 60 ? $t('sync_interval_sec', { n: String(sec) }) : $t('sync_interval_min', { n: String(sec / 60) });
  }

  function formatTime(iso: string | null): string {
    if (!iso) return $t('notes_sync_never');
    return new Date(iso).toLocaleString($locale, { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });
  }

  async function load() {
    try {
      [cfg, status] = await Promise.all([api.sync.getConfig(), api.sync.status()]);
    } catch (e) {
      error = formatError(e);
    }
  }

  async function save() {
    if (cfg) await api.sync.setConfig(cfg);
    for (const block of Object.values(blockRefs) as ({ save?: () => Promise<void> } | undefined)[]) await block?.save?.();
  }

  /** Save the form, then run `fn`; errors land in the banner. */
  async function run(fn: () => Promise<void>) {
    error = '';
    busy = true;
    try {
      await save();
      await fn();
    } catch (e) {
      const key = joinErrorKey(e);
      error = key ? $t(key) : formatError(e);
    } finally {
      busy = false;
    }
  }

  const doProbe = () =>
    run(async () => {
      probe = await api.sync.probe();
    });

  const doCreate = () =>
    run(async () => {
      status = await api.sync.createVault(passphrase);
      // The backend turned sync on: the form takes its config, or the next save would turn it off.
      cfg = await api.sync.getConfig();
      passphrase = '';
      probe = null;
      showToast($t('settings_sync_created'));
    });

  const doJoin = () =>
    run(async () => {
      status = await api.sync.joinVault(passphrase);
      cfg = await api.sync.getConfig();
      passphrase = '';
      probe = null;
      showToast($t('settings_sync_joined'));
    });

  const doSyncNow = async () => {
    if (busy || running || syncBtnState === 'syncing') return;
    error = '';
    busy = true;
    syncBtnState = 'syncing';
    clearTimeout(syncResetTimer);
    try {
      await save();
      status = await api.sync.runNow();
      if (!status.running) finishSyncBtn(status.last_error);
    } catch (e) {
      error = formatError(e);
      syncBtnState = 'idle';
    } finally {
      busy = false;
    }
  };

  function finishSyncBtn(lastError: string | null | undefined) {
    if (lastError) {
      syncBtnState = 'idle';
      return;
    }
    syncBtnState = 'done';
    clearTimeout(syncResetTimer);
    syncResetTimer = setTimeout(() => (syncBtnState = 'idle'), 1500);
  }
  const doLeave = () =>
    run(async () => {
      if (!(await ask({ title: $t('settings_sync_leave'), message: $t('settings_sync_leave_confirm'), confirmLabel: $t('settings_sync_leave') }))) return;
      status = await api.sync.leave();
      if (cfg) cfg.enabled = false;
      showToast($t('settings_sync_left'));
    });

  const toggleEnabled = () =>
    run(async () => {
      if (cfg) cfg.enabled = !cfg.enabled;
    });

  async function doSave() {
    if (busy || saveState === 'saving') return;
    error = '';
    busy = true;
    saveState = 'saving';
    clearTimeout(saveResetTimer);
    try {
      await save();
      saveState = 'saved';
      saveResetTimer = setTimeout(() => (saveState = 'idle'), 1500);
    } catch (e) {
      error = formatError(e);
      saveState = 'idle';
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    load();
    const unStatus = onSyncStatus(() =>
      api.sync.status().then((s) => {
        status = s;
        if (!s.running) {
          progress = null;
          if (syncBtnState === 'syncing') finishSyncBtn(s.last_error);
        }
      }).catch(() => {}),
    );
    const unProgress = onSyncProgress((p) => (progress = p));
    return () => {
      clearTimeout(saveResetTimer);
      clearTimeout(syncResetTimer);
      unStatus.then((f) => f());
      unProgress.then((f) => f());
    };
  });
</script>

<div class="m-page">
  <div class="m-header">
    <a class="m-ibtn" href="/settings" aria-label={$t('common_back')}>
      <Icon name="chevron-left" size={24} />
    </a>
    <h1 class="m-title">{$t('settings_sync_section')}</h1>
  </div>

  <div class="m-body">
  {#if error}
    <div class="m-error">{error}</div>
  {/if}

  {#if cfg && status}
    {#if status.joined}
      <div class="m-section">{$t('settings_sync_status')}</div>
      <div class="m-list group">
        <div class="m-row">
          <span class="m-row-label">{$t('settings_sync_vault_id')}</span>
          <span class="m-row-value mono">{status.vault_id?.slice(0, 12)}</span>
        </div>
        <div class="m-row">
          <span class="m-row-label">{$t('settings_sync_device_id')}</span>
          <span class="m-row-value mono">{status.device_id}</span>
        </div>
        <div class="m-row">
          <span class="m-row-label">{$t('settings_sync_peers')}</span>
          <span class="m-row-value">{status.peers}</span>
        </div>
        <button type="button" class="m-row" onclick={() => (devicesOpen = !devicesOpen)}>
          <span class="m-row-label">{$t('settings_sync_storage_devices')}</span>
          <span class="m-row-value">{status.storage_devices.length}</span>
          <Icon name={devicesOpen ? 'chevron-down' : 'chevron-right'} size={16} />
        </button>
        {#if devicesOpen}
          {#each status.storage_devices as d (d.id)}
            <div class="m-row device-card">
              <span class="device-name">{d.name || $t('settings_sync_device_unnamed')}</span>
              {#if d.own}<span class="device-own">{$t('settings_sync_device_id')}</span>{/if}
              <span class="device-id mono">{d.id}</span>
            </div>
          {/each}
        {/if}
        <div class="m-row">
          <span class="m-row-label">{$t('settings_sync_last_applied')}</span>
          <span class="m-row-value">{status.last_applied ?? 0}</span>
        </div>
        <div class="m-row">
          <span class="m-row-label">{$t('settings_sync_last_run')}</span>
          <span class="m-row-value">{running ? (progressLine || $t('settings_sync_running')) : formatTime(status.last_run)}</span>
        </div>
        {#if running && progress}
          <div class="m-row prog">
            <div class="bar"><div class="fill" style="width: {progress.percent}%"></div></div>
            <span class="prog-label">
              {progressLine}
              {#if progress.detail}<span class="prog-detail">{progress.detail}</span>{/if}
            </span>
          </div>
        {/if}
        {#if status.last_error && !running}
          <div class="m-row msg danger">{status.last_error}</div>
        {:else if status.last_warning}
          <div class="m-row msg warn">{status.last_warning}</div>
        {/if}
        <button class="m-row" onclick={toggleEnabled} disabled={busy}>
          <span class="m-row-label">{$t('settings_sync_enabled')}</span>
          <span class="toggle" class:on={cfg.enabled}></span>
        </button>
      </div>

      {#if status.conflicts.length > 0}
        <div class="m-section">{$t('settings_sync_conflicts')}</div>
        <p class="m-hint">{$t('settings_sync_conflicts_hint')}</p>
        <div class="m-list group">
          {#each status.conflicts as c (c.note_id)}
            <a class="m-row" href={directory.get('note')?.href?.(c.note_id, 'conflict')}>
              <Icon name="alert-triangle" size={18} />
              <span class="m-row-label">{c.title || $t('notes_untitled')}</span>
              <span class="chev"><Icon name="chevron-right" size={16} /></span>
            </a>
          {/each}
        </div>
      {/if}

      <div class="actions group">
        <button
          class="btn action-btn"
          class:btn-primary={syncVisual !== 'done'}
          class:btn-success={syncVisual === 'done'}
          class:busy-anim={syncVisual === 'syncing'}
          class:just-done={syncVisual === 'done'}
          onclick={doSyncNow}
          disabled={busy || syncVisual === 'syncing'}
        >
          <span class="action-icon" class:spin={syncVisual === 'syncing'}>
            <Icon name={syncVisual === 'done' ? 'check-circle' : 'refresh-cw'} size={16} />
          </span>
          {#if syncVisual === 'syncing'}
            {progress ? `${progress.percent}%` : $t('settings_sync_running')}
          {:else if syncVisual === 'done'}
            {$t('settings_sync_done')}
          {:else}
            {$t('settings_sync_now')}
          {/if}
        </button>
        <button class="btn btn-ghost danger" onclick={doLeave} disabled={busy || syncVisual === 'syncing'}>
          <Icon name="shield-off" size={16} />
          {$t('settings_sync_leave')}
        </button>
      </div>
    {/if}

    <div class="m-section">{$t('settings_sync_backend')}</div>
    <div class="seg backend group">
      <button class="seg-btn" class:active={cfg.backend === 'webdav'} onclick={() => (cfg!.backend = 'webdav')} disabled={status.joined}>
        {$t('settings_sync_backend_webdav')}
      </button>
      <button class="seg-btn" class:active={cfg.backend === 's3'} onclick={() => (cfg!.backend = 's3')} disabled={status.joined}>
        {$t('settings_sync_backend_s3')}
      </button>
    </div>

    {#if cfg.backend === 'webdav'}
      <div class="m-field">
        <label for="dav-url">{$t('settings_sync_webdav_url')}</label>
        <input id="dav-url" type="url" bind:value={cfg.webdav.url} placeholder="https://cloud.example.com/remote.php/dav/files/user/veydan" autocomplete="off" autocapitalize="off" />
      </div>
      <div class="m-field">
        <label for="dav-user">{$t('settings_sync_webdav_username')}</label>
        <input id="dav-user" bind:value={cfg.webdav.username} autocomplete="off" autocapitalize="off" />
      </div>
      <div class="m-field">
        <label for="dav-pass">{$t('settings_sync_webdav_password')}</label>
        <input id="dav-pass" type="password" bind:value={cfg.webdav.password} placeholder={cfg.webdav.has_password && cfg.webdav.password == null ? $t('secret_stored_placeholder') : ''} autocomplete="off" />
      </div>
    {:else}
      <div class="m-field">
        <label for="s3-endpoint">{$t('settings_sync_s3_endpoint')}</label>
        <input id="s3-endpoint" type="url" bind:value={cfg.s3.endpoint} placeholder="https://s3.example.com" autocomplete="off" autocapitalize="off" />
      </div>
      <div class="two">
        <div class="m-field">
          <label for="s3-bucket">{$t('settings_sync_s3_bucket')}</label>
          <input id="s3-bucket" bind:value={cfg.s3.bucket} autocomplete="off" autocapitalize="off" />
        </div>
        <div class="m-field">
          <label for="s3-region">{$t('settings_sync_s3_region')}</label>
          <input id="s3-region" bind:value={cfg.s3.region} placeholder="us-east-1" autocomplete="off" autocapitalize="off" />
        </div>
      </div>
      <div class="m-field">
        <label for="s3-prefix">{$t('settings_sync_s3_prefix')}</label>
        <input id="s3-prefix" bind:value={cfg.s3.prefix} autocomplete="off" autocapitalize="off" />
      </div>
      <div class="m-field">
        <label for="s3-ak">{$t('settings_sync_s3_access_key')}</label>
        <input id="s3-ak" bind:value={cfg.s3.access_key} autocomplete="off" autocapitalize="off" />
      </div>
      <div class="m-field">
        <label for="s3-sk">{$t('settings_sync_s3_secret_key')}</label>
        <input id="s3-sk" type="password" bind:value={cfg.s3.secret_key} placeholder={cfg.s3.has_secret_key && cfg.s3.secret_key == null ? $t('secret_stored_placeholder') : ''} autocomplete="off" />
      </div>
      <div class="m-list group">
        <button class="m-row" onclick={() => (cfg!.s3.path_style = !cfg!.s3.path_style)}>
          <span class="m-row-label">{$t('settings_sync_s3_path_style')}</span>
          <span class="toggle" class:on={cfg.s3.path_style}></span>
        </button>
      </div>
    {/if}

    <div class="m-field">
      <label for="interval">{$t('settings_sync_interval')}</label>
      <select id="interval" bind:value={cfg.interval_sec}>
        {#each intervals as sec (sec)}
          <option value={sec}>{intervalLabel(sec)}</option>
        {/each}
      </select>
    </div>
    <div class="m-field">
      <label for="device">{$t('settings_sync_device_name')}</label>
      <input id="device" bind:value={cfg.device_name} autocomplete="off" />
    </div>

    {#if syncsFiles}
    <div class="m-section">{$t('settings_sync_lf_section')}</div>
    <p class="m-hint">{$t('settings_sync_lf_hint')}</p>
    <div class="two">
      <div class="m-field">
        <label for="lf-chunk">{$t('settings_sync_lf_chunk')}</label>
        <input id="lf-chunk" type="number" inputmode="numeric" min={LARGE_FILE_LIMITS.chunkMib[0]} max={LARGE_FILE_LIMITS.chunkMib[1]}
          value={cfg.large_files.chunk_mib}
          oninput={(e) => (cfg!.large_files.chunk_mib = clampNum((e.currentTarget as HTMLInputElement).value, LARGE_FILE_LIMITS.chunkMib))} />
      </div>
      <div class="m-field">
        <label for="lf-par">{$t('settings_sync_lf_parallelism')}</label>
        <input id="lf-par" type="number" inputmode="numeric" min={LARGE_FILE_LIMITS.parallelism[0]} max={LARGE_FILE_LIMITS.parallelism[1]}
          value={cfg.large_files.parallelism}
          oninput={(e) => (cfg!.large_files.parallelism = clampNum((e.currentTarget as HTMLInputElement).value, LARGE_FILE_LIMITS.parallelism))} />
      </div>
    </div>
    <div class="m-list group">
      <button class="m-row" onclick={() => (cfg!.large_files.resume = !cfg!.large_files.resume)}>
        <span class="m-row-label">{$t('settings_sync_lf_resume')}</span>
        <span class="toggle" class:on={cfg.large_files.resume}></span>
      </button>
    </div>
    <p class="m-hint">{$t('settings_sync_lf_ram', { mib: String(largeFilePeakMib(cfg.large_files)) })}</p>
    {/if}

    {#each blocks as block (block.id)}
      <block.component bind:this={blockRefs[block.id]} />
    {/each}

    {#if !status.joined}
      <button class="btn btn-ghost wide" onclick={doProbe} disabled={busy}>
        <Icon name="search" size={16} />
        {$t('settings_sync_probe')}
      </button>

      {#if probe}
        <p class="probe" class:danger={probe === 'foreign'}>{$t(`settings_sync_probe_${probe}`)}</p>
        {#if probe !== 'foreign'}
          <div class="m-field">
            <label for="pass">{$t('settings_sync_passphrase')}</label>
            <input id="pass" type="password" bind:value={passphrase} placeholder={$t('settings_sync_passphrase_placeholder')} autocomplete="off" />
          </div>
          {#if probe === 'empty'}
            <button class="btn btn-primary wide" onclick={doCreate} disabled={busy || passphrase.length < 8}>
              <Icon name="plus" size={16} />
              {$t('settings_sync_create')}
            </button>
          {:else}
            <button class="btn btn-primary wide" onclick={doJoin} disabled={busy || !passphrase}>
              <Icon name="lock" size={16} />
              {$t('settings_sync_join')}
            </button>
          {/if}
        {/if}
      {/if}
    {:else}
      <button
        class="btn wide action-btn"
        class:btn-primary={saveState !== 'saved'}
        class:btn-success={saveState === 'saved'}
        class:busy-anim={saveState === 'saving'}
        class:just-done={saveState === 'saved'}
        onclick={doSave}
        disabled={busy || saveState === 'saving'}
      >
        <span class="action-icon" class:spin={saveState === 'saving'}>
          <Icon
            name={saveState === 'saving' ? 'loader' : saveState === 'saved' ? 'check-circle' : 'check'}
            size={16}
          />
        </span>
        {saveState === 'saving' ? $t('notes_saving') : saveState === 'saved' ? $t('common_saved') : $t('common_save')}
      </button>
    {/if}
  {/if}

  {#if toast}
    <div class="toast">{toast}</div>
  {/if}
  </div>
</div>

<style>
  .group { margin-bottom: var(--sp-4); }
  .device-card { flex-direction: column; align-items: stretch; gap: 2px; }
  .device-name { font-size: var(--fs-base); color: var(--text-1); }
  .device-own { font-size: var(--fs-xs); color: var(--text-3); }
  .device-id { font-size: var(--fs-sm); color: var(--text-2); word-break: break-all; }
  .backend { display: flex; width: 100%; }
  .backend .seg-btn { flex: 1; justify-content: center; }
  .two { display: grid; grid-template-columns: 1fr 1fr; gap: var(--sp-3); }
  .actions { display: flex; gap: var(--sp-2); }
  .actions .btn { flex: 1; }
  .wide { width: 100%; margin-top: var(--sp-2); }
  .msg { font-size: var(--fs-sm); word-break: break-word; }
  .danger { color: var(--danger-text); }
  .warn { color: var(--warn-text); }
  .probe { font-size: var(--fs-sm); color: var(--text-2); margin: var(--sp-3) 0; }
  .prog { flex-direction: column; align-items: stretch; gap: 6px; padding-top: 10px; padding-bottom: 12px; }
  .bar { height: 6px; background: var(--surface-3); border-radius: 999px; overflow: hidden; }
  .fill { height: 100%; background: var(--accent); border-radius: 999px; transition: width 0.2s ease; }
  .prog-label { font-size: var(--fs-sm); color: var(--text-body); }
  .prog-detail { display: block; font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-3); word-break: break-all; }
  .action-btn { justify-content: center; transition: background 0.25s ease, box-shadow 0.25s ease, transform 0.2s ease; }
  .action-btn.busy-anim { opacity: 1; }
  .action-btn.just-done { animation: action-pop 0.45s ease; }
  .action-icon { display: inline-flex; }
  @keyframes action-pop {
    0% { transform: scale(0.96); }
    40% { transform: scale(1.03); }
    100% { transform: scale(1); }
  }
</style>
