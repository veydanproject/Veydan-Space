<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import { hasHome } from '$lib/core/mobile/nav';
  import Icon from '$lib/core/Icon.svelte';
  import { theme, type Theme } from '$lib/core/theme';
  import { api as shared } from '$lib/core/api';
  import type { HostInfo } from '$lib/core/types';
  import { api, onSyncStatus, type SyncStatus } from '$lib/core/mobile/api';
  import { t, locale, type Locale } from '$lib/core/mobile/i18n';
  import { LANGUAGES } from '$lib/core/i18n';
  import { loadDefaultApp, registry, saveDefaultApp } from '$lib/core/registry';
  import PickerSheet from '$lib/core/mobile/PickerSheet.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import BugReportDialog from '$lib/core/BugReportDialog.svelte';
  import ModulesSettings from '$lib/core/ModulesSettings.svelte';
  import { formatError } from '$lib/core/utils';
  import { appLock } from '$lib/core/lock/store.svelte';
  import { product } from '$lib/core/product';
  import { updateCheck } from '$lib/core/store/update-check.svelte';

  const locales: { id: Locale; label: string }[] = LANGUAGES.map((l) => ({ id: l.code, label: l.native }));

  const demoLocales: { id: 'en' | 'ru'; label: string }[] = [
    { id: 'en', label: 'English' },
    { id: 'ru', label: 'Русский' },
  ];

  let picker = $state<'none' | 'app' | 'theme' | 'lang' | 'demoLang'>('none');
  let defaultApp = $state(loadDefaultApp());
  let info = $state<HostInfo | null>(null);
  let sync = $state<SyncStatus | null>(null);
  // The demo data is offered where a module has some (`ModuleDef.demo`), in
  // the language of the UI to begin with.
  const demoModules = registry.modules.filter((m) => m.demo);
  const hasDemo = demoModules.length > 0;
  // The demo data exists in English and Russian; every other language starts from English.
  let demoLocale = $state<'en' | 'ru'>(get(locale) === 'ru' ? 'ru' : 'en');
  /** The sections whose data the demo set replaces and "Clear app data" deletes. */
  const dataList = $derived(demoModules.map((m) => $t(m.title)).join(', '));
  // What goes with the lock's key when the demo data replaces it: each module's own sentence.
  const demoLosses = $derived(registry.active.flatMap((m) => (m.vaultResetNote ? [m.vaultResetNote] : [])));
  let demoBusy = $state(false);
  let clearBusy = $state(false);
  let pendingData = $state<'load' | 'clear' | null>(null);
  let dataMsg = $state('');
  let dataError = $state('');
  let bugOpen = $state(false);

  const themeOptions = $derived([
    { id: 'light', label: $t('settings_theme_light'), icon: 'sun' },
    { id: 'dark', label: $t('settings_theme_dark'), icon: 'moon' },
  ]);
  // Which screen opens at launch. Space: Home or one of the apps. A product
  // of one module has no Home: the choice is between the module's screens,
  // the first one unless another is chosen, and there is none with one screen.
  const withModules = registry.only === undefined;
  const appOptions = $derived([
    ...(withModules ? [{ id: '', label: $t('settings_default_home'), icon: 'home' }] : []),
    ...registry.nav().map((a) => ({ id: a.id, label: $t(a.title), icon: a.icon })),
  ]);
  const startChoice = $derived(appOptions.length > 1);
  const startTitle = $derived(withModules ? $t('settings_default_app') : $t('settings_start_screen'));
  const startValue = $derived(defaultApp || (withModules ? '' : (appOptions[0]?.id ?? '')));
  // The modules' sections, after the main list; a section that goes inside a
  // core form (`into`) is rendered there instead.
  const sections = $derived(registry.settings().filter((s) => !s.into && (s.visible?.() ?? true)));
  // The Modules section: in a product of more than one module (section 12).
  const defaultAppLabel = $derived(appOptions.find((o) => o.id === startValue)?.label ?? '');
  const themeLabel = $derived(themeOptions.find((o) => o.id === $theme)?.label ?? '');
  const localeLabel = $derived(locales.find((l) => l.id === $locale)?.label ?? '');
  const demoLocaleLabel = $derived(demoLocales.find((l) => l.id === demoLocale)?.label ?? '');
  const syncLabel = $derived(
    sync?.running ? $t('settings_sync_running')
    : sync?.enabled && sync.joined ? $t('settings_sync_connected')
    : $t('settings_sync_disconnected'),
  );
  const updateLabel = $derived(
    updateCheck.status === 'checking' ? $t('update_checking')
    : updateCheck.status === 'available' ? updateCheck.latest
    : updateCheck.status === 'upToDate' ? $t('update_up_to_date')
    : '',
  );
  const lockLabel = $derived(
    !appLock.status.enabled ? $t('settings_lock_status_off')
    : appLock.status.kind === 'pin' ? $t('lock_kind_pin')
    : $t('lock_kind_password'),
  );

  function pickApp(id: string) {
    defaultApp = id;
    saveDefaultApp(id);
  }

  /** The data file changed under the UI: every module reloads what it shows. */
  async function refreshStores() {
    await Promise.all(registry.reloaders().map((reload) => reload()));
  }

  function askLoadDemo() {
    if (demoBusy || clearBusy) return;
    pendingData = 'load';
  }

  function askClearData() {
    if (demoBusy || clearBusy) return;
    pendingData = 'clear';
  }

  async function confirmDataAction() {
    const action = pendingData;
    pendingData = null;
    if (action === 'load') await loadDemoData();
    else if (action === 'clear') await clearAppData();
  }

  async function loadDemoData() {
    if (demoBusy || clearBusy) return;
    demoBusy = true;
    dataMsg = '';
    dataError = '';
    try {
      await shared.demo.seed(demoLocale);
      await refreshStores();
      dataMsg = $t('settings_demo_done');
    } catch (e) {
      dataError = formatError(e);
    } finally {
      demoBusy = false;
    }
  }

  async function clearAppData() {
    if (demoBusy || clearBusy) return;
    clearBusy = true;
    dataMsg = '';
    dataError = '';
    try {
      await shared.app.clearData();
      await refreshStores();
      dataMsg = $t('settings_clear_done');
    } catch (e) {
      dataError = formatError(e);
    } finally {
      clearBusy = false;
    }
  }

  onMount(() => {
    void appLock.refresh();
    void shared.system.hostInfo().then((h) => (info = h));
    void api.sync.status().then((s) => (sync = s)).catch(() => {});
    const un = onSyncStatus(() => api.sync.status().then((s) => (sync = s)).catch(() => {}));
    return () => un.then((f) => f());
  });
</script>

<div class="m-page">
  <div class="m-header">
{#if hasHome()}
      <a class="m-ibtn" href="/" aria-label={$t('common_back')}><Icon name="chevron-left" size={24} /></a>
    {/if}
    <h1 class="m-title">{$t('app_settings')}</h1>
  </div>

  <div class="m-body">
  {#if updateCheck.status === 'available'}
    <div class="m-card update-card">
      <span class="update-title">{$t('update_available', { version: updateCheck.latest })}</span>
      <span class="m-sub">{$t('update_available_hint')}</span>
      <button type="button" class="btn btn-primary btn-sm update-open" onclick={() => updateCheck.open()}>
        {$t('update_open')}
      </button>
    </div>
  {/if}

  <div class="m-section">{$t('settings_main')}</div>
  <div class="m-list">
    {#if startChoice}
      <button type="button" class="m-row" onclick={() => (picker = 'app')}>
        <span class="m-row-label">{startTitle}</span>
        <span class="m-row-value">{defaultAppLabel}</span>
        <span class="chev"><Icon name="chevron-right" size={16} /></span>
      </button>
    {/if}
    <button type="button" class="m-row" onclick={() => (picker = 'theme')}>
      <span class="m-row-label">{$t('settings_theme')}</span>
      <span class="m-row-value">{themeLabel}</span>
      <span class="chev"><Icon name="chevron-right" size={16} /></span>
    </button>
    <button type="button" class="m-row" onclick={() => (picker = 'lang')}>
      <span class="m-row-label">{$t('settings_language')}</span>
      <span class="m-row-value">{localeLabel}</span>
      <span class="chev"><Icon name="chevron-right" size={16} /></span>
    </button>
  </div>

  {#if withModules}
    <div class="m-section">{$t('modules_section')}</div>
    <div class="m-list">
      <ModulesSettings mobile />
    </div>
    <p class="m-hint under-list">{$t('modules_hint')}</p>
  {/if}

  {#each sections as section (section.id)}
    <div class="m-section">{$t(section.group)}</div>
    <div class="m-list">
      <section.component />
    </div>
  {/each}

  <div class="m-section">{$t('settings_security')}</div>
  <div class="m-list">
    <a class="m-row" href="/settings/lock">
      <span class="m-row-label">{$t('settings_lock_section')} <span class="badge badge-warn">{$t('settings_sync_beta')}</span></span>
      <span class="m-row-value" class:ok={appLock.status.enabled}>{appLock.ready ? lockLabel : ''}</span>
      <span class="chev"><Icon name="chevron-right" size={16} /></span>
    </a>
  </div>

  <div class="m-section">{$t('settings_data')}</div>
  <div class="m-list">
    <a class="m-row" href="/settings/sync">
      <span class="m-row-label">{$t('settings_sync_section')}</span>
      <span class="m-row-value" class:ok={sync?.enabled && sync?.joined}>{sync ? syncLabel : ''}</span>
      <span class="chev"><Icon name="chevron-right" size={16} /></span>
    </a>
  </div>

  <div class="m-section">{$t('settings_about')}</div>
  <div class="m-list">
    <div class="m-row about-head">
      <img src="/logo.png" alt="" class="about-logo" />
      <div class="about-text">
        <span class="m-row-label">{product.name}</span>
        <span class="m-row-value mono">{info ? `${info.version} · ${info.os}/${info.arch}` : ''}</span>
      </div>
    </div>
    <button
      type="button"
      class="m-row"
      disabled={updateCheck.status === 'checking'}
      onclick={() => updateCheck.check()}
    >
      <span class="m-row-label">{$t('update_check')}</span>
      <span class="m-row-value" class:ok={updateCheck.status === 'upToDate'} class:new={updateCheck.status === 'available'}>
        {updateLabel}
      </span>
    </button>
    <button type="button" class="m-row" onclick={() => (bugOpen = true)}>
      <span class="m-row-label">{$t('settings_bug_report')}</span>
      <span class="chev"><Icon name="chevron-right" size={16} /></span>
    </button>
  </div>
  {#if updateCheck.status === 'error'}
    <p class="err-msg update-err">{$t('settings_update_failed')}</p>
  {/if}

  <BugReportDialog bind:open={bugOpen} />

  <!-- For developers: rows of the same height as every other, the destructive one marked. -->
  <div class="m-section">{$t('dev_sync_link')}</div>
  <div class="m-list">
    <a class="m-row" href="/settings/dev">
      <span class="m-row-label">{$t('dev_sync_title')}</span>
      <span class="chev"><Icon name="chevron-right" size={16} /></span>
    </a>
    {#if hasDemo}
      <button type="button" class="m-row" onclick={() => (picker = 'demoLang')}>
        <span class="m-row-label">{$t('settings_demo_locale')}</span>
        <span class="m-row-value">{demoLocaleLabel}</span>
        <span class="chev"><Icon name="chevron-right" size={16} /></span>
      </button>
      <button type="button" class="m-row" disabled={demoBusy || clearBusy} onclick={askLoadDemo}>
        <span class="m-row-label">{demoBusy ? $t('settings_demo_loading') : $t('settings_demo_load')}</span>
      </button>
      <button type="button" class="m-row danger" disabled={demoBusy || clearBusy} onclick={askClearData}>
        {clearBusy ? $t('settings_clear_clearing') : $t('settings_clear_data')}
      </button>
    {/if}
  </div>
  {#if dataMsg}<p class="ok-msg">{dataMsg}</p>{/if}
  {#if dataError}<p class="err-msg">{dataError}</p>{/if}
  </div>
</div>

{#if pendingData}
  <Dialog
    open={true}
    onclose={() => (pendingData = null)}
    title={pendingData === 'clear' ? $t('settings_clear_data') : $t('settings_demo_load')}
  >
    <p>{pendingData === 'clear' ? $t('settings_clear_confirm', { list: dataList }) : $t('settings_demo_confirm', { list: dataList })}</p>
    {#if pendingData === 'load'}
      {#each demoLosses as key (key)}<p>{$t(key)}</p>{/each}
    {/if}
    {#snippet footer()}
      <button class="btn btn-ghost btn-sm" onclick={() => (pendingData = null)}>
        {$t('settings_backup_cancel')}
      </button>
      <button
        class="btn btn-sm"
        class:btn-danger={pendingData === 'clear'}
        class:btn-primary={pendingData !== 'clear'}
        onclick={confirmDataAction}
      >
        {pendingData === 'clear' ? $t('settings_clear_data') : $t('settings_demo_load')}
      </button>
    {/snippet}
  </Dialog>
{/if}

<PickerSheet open={picker === 'app'} title={startTitle} options={appOptions} value={startValue} onclose={() => (picker = 'none')} onpick={pickApp} />
<PickerSheet open={picker === 'theme'} title={$t('settings_theme')} options={themeOptions} value={$theme} onclose={() => (picker = 'none')} onpick={(id) => ($theme = id as Theme)} />
<PickerSheet open={picker === 'lang'} title={$t('settings_language')} options={locales} value={$locale} onclose={() => (picker = 'none')} onpick={(id) => ($locale = id as Locale)} />
<PickerSheet
  open={picker === 'demoLang'}
  title={$t('settings_demo_locale')}
  options={demoLocales}
  value={demoLocale}
  onclose={() => (picker = 'none')}
  onpick={(id) => { demoLocale = id as 'en' | 'ru'; }}
/>

<style>
  .ok { color: var(--success-text); font-weight: 600; }
  .new { color: var(--accent); font-weight: 600; }
  .update-card {
    display: flex; flex-direction: column; align-items: flex-start; gap: var(--sp-2);
    margin-top: var(--sp-3); padding: var(--sp-4);
    border-color: var(--accent-border);
  }
  .update-title { font-size: 15px; font-weight: 700; color: var(--text); }
  .update-open { margin-top: var(--sp-1); }
  .update-err { margin-top: var(--sp-2); }
  .about-head { gap: var(--sp-3); }
  .about-logo { width: 40px; height: 40px; object-fit: contain; flex-shrink: 0; }
  .about-text { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .mono { font-size: 12px; }
  .under-list { margin-top: var(--sp-2); padding: 0 var(--sp-1); }
  .m-row:disabled { opacity: 0.5; }
  .ok-msg { margin: var(--sp-2) var(--sp-1) 0; font-size: 13px; color: var(--success-text); }
  .err-msg { margin: var(--sp-2) var(--sp-1) 0; font-size: 13px; color: var(--danger-text); }
</style>
