<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import { hasHome } from '$lib/core/mobile/nav';
  import Icon from '$lib/core/Icon.svelte';
  import { theme, type Theme } from '$lib/core/theme';
  import { api as shared } from '$lib/core/api';
  import { distribution } from '$lib/core/store/distribution.svelte';
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
  import { isWrongSecret } from '$lib/core/lock/secret';
  import DataConfirmFields from '$lib/core/DataConfirmFields.svelte';
  import { product } from '$lib/core/product';
  import { ABUSE_MAIL, LEGAL, openLegal } from '$lib/core/legal';
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
  /** "Clear app data" clears what these modules keep, in every build. */
  const hasData = demoModules.length > 0;
  /** The demo data only where the build loads it (`HostInfo.demo`: a debug build, or one with the feature `demo`). */
  const hasDemo = $derived(hasData && (info?.demo ?? false));
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
  // The confirmation: the lock's secret, which devices a clear reaches (DataConfirmFields).
  let dataSecret = $state('');
  let dataScope = $state<'device' | 'all'>('device');
  let dataWrong = $state(false);
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
  // The account of the product, where a module shows one: first on the page, and the way to sync.
  const accountSections = $derived(registry.accountSections());
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
    askData('load');
  }

  function askClearData() {
    if (demoBusy || clearBusy) return;
    askData('clear');
  }

  function askData(action: 'load' | 'clear') {
    dataSecret = '';
    dataScope = 'device';
    dataWrong = false;
    pendingData = action;
  }

  /** The dialog stays open on a wrong secret, and says so. */
  async function confirmDataAction() {
    const action = pendingData;
    const wrong = action === 'load' ? await loadDemoData() : action === 'clear' ? await clearAppData() : false;
    dataWrong = wrong;
    if (!wrong) pendingData = null;
  }

  /** True when the secret was not the lock's. */
  async function loadDemoData(): Promise<boolean> {
    if (demoBusy || clearBusy) return false;
    demoBusy = true;
    dataMsg = '';
    dataError = '';
    try {
      await shared.demo.seed(demoLocale, dataSecret || null);
      await refreshStores();
      dataMsg = $t('settings_demo_done');
    } catch (e) {
      if (isWrongSecret(e)) return true;
      dataError = formatError(e);
    } finally {
      demoBusy = false;
    }
    return false;
  }

  /** True when the secret was not the lock's. */
  async function clearAppData(): Promise<boolean> {
    if (demoBusy || clearBusy) return false;
    clearBusy = true;
    dataMsg = '';
    dataError = '';
    try {
      await shared.app.clearData(dataSecret || null, dataScope);
      await refreshStores();
      dataMsg = $t('settings_clear_done');
    } catch (e) {
      if (isWrongSecret(e)) return true;
      dataError = formatError(e);
    } finally {
      clearBusy = false;
    }
    return false;
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
  {#if updateCheck.status === 'available' && !distribution.fromStore}
    <div class="m-card update-card">
      <span class="update-title">{$t('update_available', { version: updateCheck.latest })}</span>
      <span class="m-sub">{$t('update_available_hint')}</span>
      <button type="button" class="btn btn-primary btn-sm update-open" onclick={() => updateCheck.open()}>
        {$t('update_open')}
      </button>
    </div>
  {/if}

  {#each accountSections as section (section.id)}
    <div class="m-list account">
      <section.component />
    </div>
  {/each}

  <div class="m-section set-h"><Icon name="settings" size={16} />{$t('settings_main')}</div>
  <div class="m-list">
    {#if startChoice}
      <button type="button" class="m-row set-row" onclick={() => (picker = 'app')}>
        <span class="set-tile"><Icon name="home" size={20} /></span>
        <span class="set-text">
          <span class="m-row-label">{startTitle}</span>
          <span class="m-row-value">{defaultAppLabel}</span>
        </span>
        <span class="chev"><Icon name="chevron-right" size={16} /></span>
      </button>
    {/if}
    <button type="button" class="m-row set-row" onclick={() => (picker = 'theme')}>
        <span class="set-tile"><Icon name="sun" size={20} /></span>
        <span class="set-text">
          <span class="m-row-label">{$t('settings_theme')}</span>
          <span class="m-row-value">{themeLabel}</span>
        </span>
      <span class="chev"><Icon name="chevron-right" size={16} /></span>
    </button>
    <button type="button" class="m-row set-row" onclick={() => (picker = 'lang')}>
        <span class="set-tile"><Icon name="globe" size={20} /></span>
        <span class="set-text">
          <span class="m-row-label">{$t('settings_language')}</span>
          <span class="m-row-value">{localeLabel}</span>
        </span>
      <span class="chev"><Icon name="chevron-right" size={16} /></span>
    </button>
  </div>

  {#if withModules}
    <div class="m-section set-h"><Icon name="layout-grid" size={16} />{$t('modules_section')}</div>
    <div class="m-list">
      <ModulesSettings mobile />
    </div>
    <p class="m-hint under-list">{$t('modules_hint')}</p>
  {/if}

  {#each sections as section (section.id)}
    <div class="m-section set-h"><Icon name="layers" size={16} />{$t(section.group)}</div>
    <div class="m-list">
      <section.component />
    </div>
  {/each}

  <div class="m-section set-h"><Icon name="lock" size={16} />{$t('settings_security')}</div>
  <div class="m-list">
    <a class="m-row set-row" href="/settings/lock">
        <span class="set-tile"><Icon name="lock" size={20} /></span>
        <span class="set-text">
          <span class="m-row-label">{$t('settings_lock_section')} <span class="badge badge-warn">{$t('settings_sync_beta')}</span></span>
          <span class="m-row-value" class:ok={appLock.status.enabled}>{appLock.ready ? lockLabel : ''}</span>
        </span>
      <span class="chev"><Icon name="chevron-right" size={16} /></span>
    </a>
  </div>

  <!-- With an account on the page, its card leads to sync. -->
  {#if accountSections.length === 0}
    <div class="m-section set-h"><Icon name="refresh-cw" size={16} />{$t('settings_data')}</div>
    <div class="m-list">
      <a class="m-row set-row" href="/settings/sync">
        <span class="set-tile"><Icon name="refresh-cw" size={20} /></span>
        <span class="set-text">
          <span class="m-row-label">{$t('settings_sync_section')}</span>
          <span class="m-row-value" class:ok={sync?.enabled && sync?.joined}>{sync ? syncLabel : ''}</span>
        </span>
        <span class="chev"><Icon name="chevron-right" size={16} /></span>
      </a>
    </div>
  {/if}

  <div class="m-section set-h"><Icon name="info" size={16} />{$t('settings_about')}</div>
  <div class="m-list">
    <div class="m-row about-head">
      <img src="/logo.png" alt="" class="about-logo" />
      <div class="about-text">
        <span class="m-row-label">{product.name}</span>
        <span class="m-row-value mono">{info ? `${info.version} · ${info.os}/${info.arch}` : ''}</span>
      </div>
    </div>
    {#if !distribution.fromStore}
    <button
      type="button"
      class="m-row set-row"
      disabled={updateCheck.status === 'checking'}
      onclick={() => updateCheck.check()}
    >
        <span class="set-tile"><Icon name="download" size={20} /></span>
        <span class="set-text">
          <span class="m-row-label">{$t('update_check')}</span>
          <span class="m-row-value" class:ok={updateCheck.status === 'upToDate'} class:new={updateCheck.status === 'available'}>{updateLabel}</span>
        </span>
    </button>
    {/if}
    <button type="button" class="m-row set-row" onclick={() => (bugOpen = true)}>
        <span class="set-tile"><Icon name="alert-triangle" size={20} /></span>
        <span class="set-text">
          <span class="m-row-label">{$t('settings_bug_report')}</span>
        </span>
      <span class="chev"><Icon name="chevron-right" size={16} /></span>
    </button>
    <button type="button" class="m-row set-row" onclick={() => openLegal(LEGAL.privacy)}>
        <span class="set-tile"><Icon name="shield" size={20} /></span>
        <span class="set-text">
          <span class="m-row-label">{$t('settings_about_privacy')}</span>
        </span>
      <span class="chev"><Icon name="external-link" size={16} /></span>
    </button>
    <button type="button" class="m-row set-row" onclick={() => openLegal(LEGAL.terms)}>
        <span class="set-tile"><Icon name="file-text" size={20} /></span>
        <span class="set-text">
          <span class="m-row-label">{$t('settings_about_terms')}</span>
        </span>
      <span class="chev"><Icon name="external-link" size={16} /></span>
    </button>
    <button type="button" class="m-row set-row" onclick={() => openLegal(LEGAL.childSafety)}>
        <span class="set-tile"><Icon name="shield" size={20} /></span>
        <span class="set-text">
          <span class="m-row-label">{$t('settings_about_child_safety')}</span>
        </span>
      <span class="chev"><Icon name="external-link" size={16} /></span>
    </button>
    <button type="button" class="m-row set-row" onclick={() => openLegal(LEGAL.deleteAccount)}>
        <span class="set-tile"><Icon name="trash" size={20} /></span>
        <span class="set-text">
          <span class="m-row-label">{$t('settings_about_delete_account')}</span>
        </span>
      <span class="chev"><Icon name="external-link" size={16} /></span>
    </button>
    <button type="button" class="m-row set-row" onclick={() => openLegal(ABUSE_MAIL)}>
        <span class="set-tile"><Icon name="alert-triangle" size={20} /></span>
        <span class="set-text">
          <span class="m-row-label">{$t('settings_about_abuse')}</span>
        </span>
      <span class="chev"><Icon name="external-link" size={16} /></span>
    </button>
  </div>
  {#if updateCheck.status === 'error' && !distribution.fromStore}
    <p class="err-msg update-err">{$t('settings_update_failed')}</p>
  {/if}

  <BugReportDialog bind:open={bugOpen} />

  <!-- For developers: rows of the same height as every other, the destructive one marked. -->
  <div class="m-section set-h"><Icon name="code" size={16} />{$t('dev_sync_link')}</div>
  <div class="m-list">
    <a class="m-row set-row" href="/settings/dev">
        <span class="set-tile"><Icon name="code" size={20} /></span>
        <span class="set-text">
          <span class="m-row-label">{$t('dev_sync_title')}</span>
        </span>
      <span class="chev"><Icon name="chevron-right" size={16} /></span>
    </a>
    {#if hasDemo}
      <button type="button" class="m-row set-row" onclick={() => (picker = 'demoLang')}>
        <span class="set-tile"><Icon name="globe" size={20} /></span>
        <span class="set-text">
          <span class="m-row-label">{$t('settings_demo_locale')}</span>
          <span class="m-row-value">{demoLocaleLabel}</span>
        </span>
        <span class="chev"><Icon name="chevron-right" size={16} /></span>
      </button>
      <button type="button" class="m-row set-row" disabled={demoBusy || clearBusy} onclick={askLoadDemo}>
        <span class="set-tile"><Icon name="file-plus" size={20} /></span>
        <span class="set-text">
          <span class="m-row-label">{demoBusy ? $t('settings_demo_loading') : $t('settings_demo_load')}</span>
        </span>
      </button>
    {/if}
    {#if hasData}
      <button type="button" class="m-row set-row set-danger" disabled={demoBusy || clearBusy} onclick={askClearData}>
        <span class="set-tile"><Icon name="trash" size={20} /></span>
        <span class="set-text">
          <span class="m-row-label">{clearBusy ? $t('settings_clear_clearing') : $t('settings_clear_data')}</span>
        </span>
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
    <DataConfirmFields
      clear={pendingData === 'clear'}
      bind:secret={dataSecret}
      bind:scope={dataScope}
      wrong={dataWrong}
      disabled={demoBusy || clearBusy}
    />
    {#snippet footer()}
      <button class="btn btn-ghost btn-sm" onclick={() => (pendingData = null)}>
        {$t('settings_backup_cancel')}
      </button>
      <button
        class="btn btn-sm"
        class:btn-danger={pendingData === 'clear'}
        class:btn-primary={pendingData !== 'clear'}
        disabled={demoBusy || clearBusy || (appLock.status.enabled && !dataSecret)}
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
  .account { margin-top: var(--sp-3); }
  .new { color: var(--accent-text); font-weight: 600; }
  .update-card {
    display: flex; flex-direction: column; align-items: flex-start; gap: var(--sp-2);
    margin-top: var(--sp-3); padding: var(--sp-4);
    border-color: var(--accent-border);
  }
  .update-title { font-size: var(--fs-body); font-weight: 700; color: var(--text); }
  .update-open { margin-top: var(--sp-1); }
  .update-err { margin-top: var(--sp-2); }
  .about-head { gap: var(--sp-3); }
  .about-logo { width: 40px; height: 40px; object-fit: contain; flex-shrink: 0; }
  .about-text { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .mono { font-size: var(--fs-xs); }
  .under-list { margin-top: var(--sp-2); padding: 0 var(--sp-1); }
  .m-row:disabled { opacity: 0.5; }

  /* The rows of the design kit: a tile with the icon, the name, its value under it. */
  .set-h { display: flex; align-items: center; gap: var(--sp-2); }
  .set-h :global(.icon) { color: var(--text-2); }
  .m-list:has(.set-row) { padding: var(--sp-1); }
  .set-row { gap: var(--sp-3); min-height: 60px; padding: var(--sp-2) var(--sp-3); border-bottom: 0; border-radius: var(--radius-lg); }
  .set-tile {
    display: inline-flex; align-items: center; justify-content: center; flex-shrink: 0;
    width: 36px; height: 36px; border-radius: var(--radius-md);
    background: var(--m-seg); color: var(--text-soft);
  }
  .set-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .set-text .m-row-label { flex: none; font-weight: var(--fw-medium); }
  .set-text .m-row-value { font-size: var(--fs-sm); overflow: hidden; text-overflow: ellipsis; }
  .set-danger { color: var(--danger-text); }
  .set-danger .set-tile { background: var(--danger-bg); color: var(--danger-text); }
  .ok-msg { margin: var(--sp-2) var(--sp-1) 0; font-size: var(--fs-sm); color: var(--success-text); }
  .err-msg { margin: var(--sp-2) var(--sp-1) 0; font-size: var(--fs-sm); color: var(--danger-text); }
</style>
