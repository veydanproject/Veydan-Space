<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import { LANGUAGES, locale, t, taglineOf, type Locale, type TranslationKey } from '$lib/core/i18n';
  import {
    theme,
    themeCustom,
    resolvedOverride,
    hasThemeOverrides,
    setThemeOverride,
    resetThemeOverrides,
  } from '$lib/core/theme';
  import ColorField from '$lib/core/ui/ColorField.svelte';
  import { inspectorApp } from '$lib/core/inspector/inspector.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { api } from '$lib/core/api';
  import { formatError } from '$lib/core/utils';
  import { updaterStore } from '$lib/core/store/updater.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import LockSetupForm from '$lib/core/lock/LockSetupForm.svelte';
  import { appLock } from '$lib/core/lock/store.svelte';
  import { isWrongSecret } from '$lib/core/lock/secret';
  import DataConfirmFields from '$lib/core/DataConfirmFields.svelte';
  import SyncSettings from '$lib/core/sync/SyncSettings.svelte';
  import BugReportDialog from '$lib/core/BugReportDialog.svelte';
  import HotkeySettings from '$lib/core/desktop/HotkeySettings.svelte';
  import ModulesSettings from '$lib/core/ModulesSettings.svelte';
  import SettingsNav, { type SettingsNavGroup } from '$lib/core/desktop/SettingsNav.svelte';
  import { formatCommand, keybindingOverrides } from '$lib/core/keybindings';
  import { registry } from '$lib/core/registry';
  import { product } from '$lib/core/product';
  import { ABUSE_MAIL, LEGAL, openLegal } from '$lib/core/legal';
  import type { Key, SettingsSection } from '$lib/core/module';

  const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

  // The page is the core's cards — general, security, data, about — with the
  // modules' cards between them (platform-spec 11.5): a module's card names
  // its group and its order; cards of one group form one block of the nav,
  // and blocks are ordered by the smallest order among their cards. A
  // product of more than one module opens with the Modules section (12); a
  // module that is switched off has no cards.
  const withModules = registry.only === undefined;
  const CORE_BLOCKS: { group: Key; order: number; ids: string[] }[] = [
    {
      group: 'settings_group_general',
      order: 20,
      ids: [...(withModules ? ['modules'] : []), 'language', 'theme', 'devtools', 'hotkeys', 'tray'],
    },
    { group: 'settings_group_security', order: 40, ids: ['lock'] },
    { group: 'settings_group_data', order: 50, ids: ['sync'] },
    { group: 'settings_group_about', order: 70, ids: ['updates', 'about'] },
  ];
  const moduleSections = $derived(
    registry.settings().filter((sec) => !sec.into && (sec.visible?.() ?? true)).sort((x, y) => x.order - y.order),
  );
  interface Block { group: Key; order: number; sections: SettingsSection[] }
  /**
   * A module's cards that join a core block: before the core's own cards when
   * their order is not above the block's (the backup service's card, before
   * Sync), after them otherwise.
   */
  const joining = (group: Key, before: boolean): Block => {
    const order = CORE_BLOCKS.find((c) => c.group === group)?.order ?? 0;
    return { group, order, sections: moduleSections.filter((sec) => sec.group === group && (sec.order <= order) === before) };
  };
  /** Blocks made of module cards alone — Browser, Notes — by order. */
  const moduleBlocks = $derived.by(() => {
    const blocks: Block[] = [];
    for (const sec of moduleSections) {
      if (CORE_BLOCKS.some((c) => c.group === sec.group)) continue;
      const block = blocks.find((x) => x.group === sec.group);
      if (block) block.sections.push(sec);
      else blocks.push({ group: sec.group, order: sec.order, sections: [sec] });
    }
    return blocks;
  });
  /** Module blocks between two core blocks (`hi` undefined: after the last). */
  const blocksBetween = (lo: number, hi?: number) => moduleBlocks.filter((x) => x.order > lo && (hi === undefined || x.order < hi));
  const navGroups = $derived<SettingsNavGroup[]>(
    [...CORE_BLOCKS.map((c): Block => ({ group: c.group, order: c.order, sections: [] })), ...moduleBlocks]
      .sort((x, y) => x.order - y.order)
      .map((block) => {
        const core = CORE_BLOCKS.find((c) => c.group === block.group);
        const item = (sec: SettingsSection) => ({ id: sec.id, label: $t(sec.title as TranslationKey), icon: sec.icon ?? 'settings' });
        return {
          label: $t(block.group as TranslationKey),
          items: core
            ? [
                ...joining(core.group, true).sections.map(item),
                ...core.ids.map((id) => ({ id, label: coreLabel(id), icon: CORE_ICONS[id] ?? 'settings' })),
                ...joining(core.group, false).sections.map(item),
              ]
            : block.sections.map(item),
        };
      }),
  );
  const CORE_ICONS: Record<string, string> = {
    modules: 'layout-grid', language: 'language', theme: 'sun', devtools: 'code', hotkeys: 'keyboard',
    tray: 'arrows-minimize', lock: 'lock', sync: 'refresh-cw', updates: 'download', about: 'info',
  };
  function coreLabel(id: string): string {
    switch (id) {
      case 'modules': return $t('modules_section');
      case 'language': return $t('settings_section_language');
      case 'theme': return $t('settings_section_theme');
      case 'devtools': return $t('inspector_developer_tools');
      case 'hotkeys': return $t('hotkey_section');
      case 'tray': return $t('settings_tray_section');
      case 'lock': return $t('settings_lock_section');
      case 'sync': return $t('settings_sync_section');
      case 'updates': return $t('settings_update_section');
      default: return $t('settings_section_about');
    }
  }
  let activeSection = $state(blocksBetween(-Infinity, 20)[0]?.sections[0]?.id ?? (withModules ? 'modules' : 'language'));
  let scrollEl = $state<HTMLElement | null>(null);
  const navItems = $derived(navGroups.flatMap((g) => g.items));
  const activeLabel = $derived(navItems.find((it) => it.id === activeSection)?.label ?? '');
  const activeBadge = $derived(moduleSections.find((sec) => sec.id === activeSection)?.badge);

  /**
   * One section a page (the design kit’s settings): the choice shows its
   * card, the others stay mounted and hidden, and the address keeps it
   * (`/settings#updates`, the palette’s "Check for updates").
   */
  function selectSection(id: string) {
    activeSection = id;
    if (scrollEl) scrollEl.scrollTop = 0;
    if (location.hash !== `#${id}`) history.replaceState(history.state, '', `#${id}`);
  }
  function sectionFromHash() {
    const wanted = location.hash.slice(1);
    if (wanted && navItems.some((it) => it.id === wanted)) selectSection(wanted);
  }
  /** What is not the chosen section is hidden (a rule of its own: the cards come and go with the modules). */
  const hideOthers = $derived(
    `<style>.settings-main > [id]:not([id="${CSS.escape(activeSection)}"]) { display: none !important; }</style>`,
  );

  const ACCENT_PRESETS = ['#7066c6', '#8b7bff', '#60a5fa', '#2dd4bf', '#f472b6', '#34d399'];

  /** The user's choice: shown here, remembered by this device and saved for the extension and the other computers (it syncs). */
  function chooseLanguage(code: Locale) {
    locale.set(code);
    if (isTauri) api.settings.setLocale(code).catch((e) => console.error('app_locale_set failed', e));
  }

  // The product's own repository (products.json `repo`): releases, licence files.
  const REPO_URL: string = product.repo;
  const RELEASES_URL = `${REPO_URL}/releases/latest`;
  const LICENSE_URL = 'https://polyformproject.org/licenses/perimeter/1.0.1';
  // What the product's modules are built on (the browser's Camoufox): their links of About.
  const aboutLinks = $derived(registry.active.flatMap((m) => m.about ?? []));
  const summaryUrl = REPO_URL ? `${REPO_URL}/blob/main/LICENSE-SUMMARY.md` : '';
  const thirdPartyUrl = REPO_URL ? `${REPO_URL}/blob/main/THIRD-PARTY-LICENSES.md` : '';

  async function openExternal(url: string) {
    if (!url) return;
    if (isTauri) {
      await api.system.openUrl(url).catch((e) => console.error(e));
    } else {
      window.open(url, '_blank', 'noopener');
    }
  }

  let unlisteners: (() => void)[] = [];

  let appVersion = $state('');
  let bugOpen = $state(false);
  // System tray
  let trayMinimize = $state(false);
  let trayClose = $state(false);
  let trayStartHidden = $state(false);

  // Demo / clear data
  // The demo data is offered where a module has some (`ModuleDef.demo`), in
  // the language of the UI to begin with.
  const demoModules = registry.modules.filter((m) => m.demo);
  /** "Clear app data" clears what these modules keep, in every build. */
  const hasData = demoModules.length > 0;
  /** The demo data only where the build loads it (`HostInfo.demo`: a debug build, or one with the feature `demo`). */
  let demoBuild = $state(false);
  const hasDemo = $derived(hasData && demoBuild);
  /** The sections whose data the demo set replaces and "Clear app data" deletes. */
  const dataList = $derived(demoModules.map((m) => $t(m.title as TranslationKey)).join(', '));
  // The demo data exists in English and Russian; every other language starts from English.
  let demoLocale = $state<'en' | 'ru'>(get(locale) === 'ru' ? 'ru' : 'en');
  // What goes with the lock's key when the demo data replaces it: each module's own sentence.
  const demoLosses = $derived(registry.active.flatMap((m) => (m.vaultResetNote ? [m.vaultResetNote] : [])));
  let demoBusy = $state(false);
  let pendingData = $state<'load' | 'clear' | null>(null);
  let clearBusy = $state(false);
  let dataMsg = $state('');
  let dataError = $state('');
  // The confirmation: the lock's secret, which devices a clear reaches (DataConfirmFields).
  let dataSecret = $state('');
  let dataScope = $state<'device' | 'all'>('device');
  let dataWrong = $state(false);

  async function saveTray() {
    if (!isTauri) return;
    try {
      await api.settings.setTray({
        minimize_to_tray: trayMinimize,
        close_to_tray: trayClose,
        start_hidden: trayStartHidden,
      });
    } catch (e) {
      console.error(e);
    }
  }

  onMount(async () => {
    // A link to one section (`/settings#updates`, the palette's "Check for updates").
    sectionFromHash();
    window.addEventListener('hashchange', sectionFromHash);
    unlisteners.push(() => window.removeEventListener('hashchange', sectionFromHash));

    if (isTauri) {
      const { getVersion } = await import('@tauri-apps/api/app');
      appVersion = await getVersion().catch(() => '');
    }

    // Load tray settings
    if (isTauri) {
      try {
        const tray = await api.settings.getTray();
        trayMinimize = tray.minimize_to_tray;
        trayClose = tray.close_to_tray;
        trayStartHidden = tray.start_hidden;
      } catch {}
      demoBuild = (await api.system.hostInfo().catch(() => null))?.demo ?? false;
    }

  });

  onDestroy(() => unlisteners.forEach(fn => fn()));

  function formatMb(bytes: number) {
    return (bytes / 1024 / 1024).toFixed(0) + ' MB';
  }

  /** The data file changed under the UI: every module reloads what it shows. */
  async function refreshCatalogStores() {
    await Promise.all(registry.reloaders().map((reload) => reload()));
  }

  function askLoadDemo() {
    if (demoBusy || clearBusy) return;
    if (!isTauri) {
      dataError = formatError('Not running in app');
      return;
    }
    askData('load');
  }

  function askClearData() {
    if (demoBusy || clearBusy) return;
    if (!isTauri) {
      dataError = formatError('Not running in app');
      return;
    }
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
    if (!isTauri) {
      dataError = formatError('Not running in app');
      return false;
    }
    demoBusy = true;
    dataMsg = '';
    dataError = '';
    try {
      await api.demo.seed(demoLocale, dataSecret || null);
      await refreshCatalogStores();
      dataMsg = $t('settings_demo_done');
      setTimeout(() => (dataMsg = ''), 4000);
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
    if (!isTauri) {
      dataError = formatError('Not running in app');
      return false;
    }
    clearBusy = true;
    dataMsg = '';
    dataError = '';
    try {
      await api.app.clearData(dataSecret || null, dataScope);
      await refreshCatalogStores();
      dataMsg = $t('settings_clear_done');
      setTimeout(() => (dataMsg = ''), 4000);
    } catch (e) {
      if (isWrongSecret(e)) return true;
      dataError = formatError(e);
    } finally {
      clearBusy = false;
    }
    return false;
  }

</script>

{#snippet moduleCards(blocks: Block[])}
  {#each blocks as block (block.group)}
    {#each block.sections as section (section.id)}
      {#if section.bare}
      <div class="bare" id={section.id}>
        {#if section.hint}<p class="muted bare-hint">{$t(section.hint as TranslationKey)}</p>{/if}
        <section.component />
      </div>
      {:else}
      <div class="card" id={section.id}>
        <div class="card-title">
          {$t(section.title as TranslationKey)}
          {#if section.badge}<span class="badge badge-warn">{$t(section.badge as TranslationKey)}</span>{/if}
        </div>
        {#if section.hint}<p class="muted">{$t(section.hint as TranslationKey)}</p>{/if}
        <section.component />
      </div>
      {/if}
    {/each}
  {/each}
{/snippet}

<svelte:head>{@html hideOthers}</svelte:head>

<div class="settings-layout">
  <aside class="settings-side">
    <h1 class="side-title">{$t('settings_title')}</h1>
    <SettingsNav groups={navGroups} active={activeSection} onselect={selectSection} />
  </aside>

  <section class="settings-pane">
  <header class="pane-head">
    <h2 class="pane-title">{activeLabel}</h2>
    {#if activeBadge}<span class="badge badge-warn">{$t(activeBadge as TranslationKey)}</span>{/if}
  </header>
  <div class="pane-scroll" bind:this={scrollEl}>
  <div class="settings-main">

  <!-- The modules' cards before the general block: the browser's Camoufox. -->
  {@render moduleCards(blocksBetween(-Infinity, 20))}

  {@render moduleCards([joining('settings_group_general', true)])}

  {#if withModules}
    <!-- Modules -->
    <!-- The hint over the list; the list is its own card of rows, each row the module's switch. -->
    <div class="bare" id="modules">
      <p class="muted bare-hint">{$t('modules_hint')}</p>
      <ModulesSettings />
    </div>
  {/if}

  <!-- Language -->
  <!-- The hint over the card; the languages are one choice (a radio group), the chosen one marked by a check. -->
  <div class="bare" id="language">
    <p class="muted bare-hint" id="language-label">{$t('settings_language_label')}</p>
    <div class="card lang-options" role="radiogroup" aria-labelledby="language-label">
      {#each LANGUAGES as lang (lang.code)}
        <button
          type="button"
          class="lang-btn"
          class:active={$locale === lang.code}
          role="radio"
          aria-checked={$locale === lang.code}
          lang={lang.tag}
          onclick={() => chooseLanguage(lang.code)}
        >
          <span class="lang-code">{lang.code}</span>
          <span class="lang-native">{lang.native}</span>
          {#if $locale === lang.code}<span class="lang-check"><Icon name="check" size={18} /></span>{/if}
        </button>
      {/each}
    </div>
  </div>

  <!-- Theme -->
  <!-- The two themes are one choice (a radio group) of tiles, the chosen one checked; under a line the accent, its reset in the label's row. -->
  <div class="card theme-card" id="theme">
    <div class="card-title">{$t('settings_section_theme')}</div>
    <div class="theme-options" role="radiogroup" aria-label={$t('settings_section_theme')}>
      {#each [{ id: 'dark', icon: 'moon', label: 'settings_theme_dark' }, { id: 'light', icon: 'sun', label: 'settings_theme_light' }] as const as o (o.id)}
        <button
          type="button"
          class="theme-opt"
          class:active={$theme === o.id}
          role="radio"
          aria-checked={$theme === o.id}
          onclick={() => theme.set(o.id)}
        >
          <span class="theme-icon"><Icon name={o.icon} size={18} /></span>
          <span class="theme-name">{$t(o.label)}</span>
          {#if $theme === o.id}<span class="lang-check"><Icon name="check" size={18} /></span>{/if}
        </button>
      {/each}
    </div>
    <div class="theme-customize">
      <ColorField
        label={$t('settings_theme_accent')}
        value={resolvedOverride($themeCustom, $theme, 'accent')}
        presets={ACCENT_PRESETS}
        onchange={(hex) => setThemeOverride('accent', hex)}
      >
        {#snippet action()}
          <button
            type="button"
            class="btn btn-ghost btn-sm"
            disabled={!hasThemeOverrides($themeCustom, $theme)}
            onclick={resetThemeOverrides}
          >
            {$t('settings_theme_reset')}
          </button>
        {/snippet}
      </ColorField>
    </div>
  </div>

  <!-- Developer Tools -->
  <!-- One row, the whole of it the inspector's switch; its hotkey as a key chip. -->
  <div class="card switch-card" id="devtools">
    <div class="card-title">{$t('inspector_developer_tools')}</div>
    <button
      type="button"
      class="switch-row"
      role="switch"
      aria-checked={inspectorApp.enabled}
      onclick={() => inspectorApp.toggle()}
    >
      <span class="switch-words">
        <span class="switch-title">{$t('inspector_toggle')}</span>
        <span class="switch-sub">
          {#each $t('inspector_hotkey_hint').split('{keys}') as part, i}
            {#if i > 0}
              {@const keys = formatCommand('app.inspector', $keybindingOverrides)}
              {#if keys}<kbd class="keycap">{keys}</kbd>{:else}{$t('hotkey_unbound')}{/if}
            {/if}
            {part}
          {/each}
        </span>
      </span>
      <span class="toggle" class:on={inspectorApp.enabled} aria-hidden="true"></span>
    </button>
  </div>

  <div id="hotkeys">
    <HotkeySettings />
  </div>

  <!-- System tray -->
  <!-- Each row the whole of it a switch, as the developer tools' row. -->
  <div class="card switch-card" id="tray">
    <div class="card-title">{$t('settings_tray_section')}</div>
    {#snippet trayRow(on: boolean, title: string, hint: string, flip: () => void)}
      <button type="button" class="switch-row" role="switch" aria-checked={on} disabled={!isTauri} onclick={() => { flip(); saveTray(); }}>
        <span class="switch-words">
          <span class="switch-title">{title}</span>
          <span class="switch-sub">{hint}</span>
        </span>
        <span class="toggle" class:on aria-hidden="true"></span>
      </button>
    {/snippet}
    {@render trayRow(trayMinimize, $t('settings_tray_minimize'), $t('settings_tray_minimize_hint'), () => (trayMinimize = !trayMinimize))}
    {@render trayRow(trayClose, $t('settings_tray_close'), $t('settings_tray_close_hint'), () => (trayClose = !trayClose))}
    {@render trayRow(trayStartHidden, $t('settings_tray_start_hidden'), $t('settings_tray_start_hidden_hint'), () => (trayStartHidden = !trayStartHidden))}
  </div>

  {@render moduleCards([joining('settings_group_general', false), ...blocksBetween(20, 40)])}

  {@render moduleCards([joining('settings_group_security', true)])}

  <!-- App lock -->
  <!-- The hint over the form; the form is its own cards. -->
  <div class="bare" id="lock">
    <p class="muted bare-hint">{$t('settings_lock_hint')}</p>
    <LockSetupForm />
  </div>

  {@render moduleCards([joining('settings_group_security', false), ...blocksBetween(40, 50)])}

  <!-- The backup service's card -->
  {@render moduleCards([joining('settings_group_data', true)])}

  <!-- Sync (beta) -->
  <!-- The hint over the blocks; the sync's form is its own cards. -->
  <div class="bare" id="sync">
    <p class="muted bare-hint">{$t('settings_sync_hint')}</p>
    <SyncSettings />
  </div>

  {@render moduleCards([joining('settings_group_data', false), ...blocksBetween(50, 70)])}

  {@render moduleCards([joining('settings_group_about', true)])}

  <!-- App updates -->
  <!-- One card: the version and what the last check said, the actions on the right; under a line, the work under way. -->
  <div class="card update-card" id="updates">
    <div class="card-title">{$t('settings_update_section')}</div>

    <div class="update-head">
      <span class="update-words">
        <span class="update-key">{$t('settings_about_version')}</span>
        <span class="update-version mono">{appVersion ? `v${appVersion}` : '—'}</span>
        {#if updaterStore.status === 'upToDate'}
          <span class="update-line ok" role="status">{$t('settings_update_up_to_date')}</span>
        {:else if updaterStore.status === 'available' || updaterStore.status === 'downloading' || updaterStore.status === 'installing'}
          {#if updaterStore.status === 'available' || updaterStore.version}
            <span class="update-line new" role="status">{$t('settings_update_available', { version: updaterStore.version })}</span>
          {/if}
        {/if}
      </span>
      <span class="update-acts">
        <button
          type="button"
          class="btn btn-ghost"
          disabled={updaterStore.status === 'checking' ||
            updaterStore.status === 'downloading' ||
            updaterStore.status === 'installing'}
          onclick={() => updaterStore.check(false)}
        >
          {updaterStore.status === 'checking'
            ? $t('settings_update_checking')
            : $t('settings_update_check')}
        </button>
        {#if updaterStore.supported && (updaterStore.status === 'available' || updaterStore.status === 'downloading' || updaterStore.status === 'installing')}
          <button type="button" class="btn btn-primary" disabled={updaterStore.status !== 'available'} onclick={() => updaterStore.install()}>
            {$t('settings_update_install')}
          </button>
        {/if}
      </span>
    </div>

    {#if updaterStore.status === 'available' && !updaterStore.supported}
      <div class="update-work">
        <div class="update-note">
          <Icon name="info" size={18} />
          <span class="update-note-words">
            <span>{$t('settings_update_unsupported')}</span>
            <button type="button" class="btn btn-ghost btn-sm" onclick={() => openExternal(RELEASES_URL)}>
              <Icon name="external-link" size={18} />
              {$t('settings_update_open_releases')}
            </button>
          </span>
        </div>
      </div>
    {:else if updaterStore.status === 'downloading'}
      <div class="update-work">
        <div class="progress-wrap" role="status">
          <div class="progress-bar">
            <div class="progress-fill" style="width: {updaterStore.progress}%"></div>
          </div>
          <span class="progress-label">
            {$t('settings_update_downloading')} · {updaterStore.progress}%
          </span>
        </div>
      </div>
    {:else if updaterStore.status === 'installing'}
      <div class="update-work"><p class="muted small" role="status">{$t('settings_update_installing')}</p></div>
    {:else if updaterStore.status === 'error'}
      <!-- The plugin's own text is in the console (store/updater.svelte.ts). -->
      <div class="update-work">
        <div class="update-fail" role="alert"><Icon name="alert-triangle" size={18} /><span>{$t('settings_update_failed')}</span></div>
      </div>
    {/if}
  </div>

  <!-- About -->
  <!-- The program's card (its name, version, licence and the licence's note); the rules as a card of rows
       under a caption; the bug report and the project's links as another; the copyright under them. -->
  <div class="bare" id="about">
    <div class="card about-card">
      <div class="about-head">
        <span class="about-logo">
          <img src="/logo.png" alt="" />
        </span>
        <div class="about-head-text">
          <div class="about-app">{product.name}</div>
          <div class="about-tagline">{taglineOf($locale)}</div>
        </div>
      </div>

      <div class="about-facts">
        <div class="about-fact">
          <span class="about-key">{$t('settings_about_version')}</span>
          <span class="about-value mono">{appVersion ? `v${appVersion}` : '—'}</span>
        </div>
        <div class="about-fact">
          <span class="about-key">{$t('settings_about_license')}</span>
          <span class="about-value mono">PolyForm Perimeter 1.0.1</span>
        </div>
      </div>

      <p class="about-note">{$t('settings_about_license_note')}</p>
    </div>

    <section class="about-group" aria-labelledby="about-legal">
      <div class="about-caption" id="about-legal">{$t('settings_about_legal')}</div>
      <div class="card about-list">
        <button type="button" class="about-row" onclick={() => openLegal(LEGAL.privacy)}>
          <span class="about-row-label">{$t('settings_about_privacy')}</span>
          <Icon name="external-link" size={18} />
        </button>
        <button type="button" class="about-row" onclick={() => openLegal(LEGAL.terms)}>
          <span class="about-row-label">{$t('settings_about_terms')}</span>
          <Icon name="external-link" size={18} />
        </button>
        <button type="button" class="about-row" onclick={() => openLegal(LEGAL.childSafety)}>
          <span class="about-row-label">{$t('settings_about_child_safety')}</span>
          <Icon name="external-link" size={18} />
        </button>
        <button type="button" class="about-row" onclick={() => openLegal(LEGAL.deleteAccount)}>
          <span class="about-row-label">{$t('settings_about_delete_account')}</span>
          <Icon name="external-link" size={18} />
        </button>
        <button type="button" class="about-row" onclick={() => openLegal(ABUSE_MAIL)}>
          <span class="about-row-label">{$t('settings_about_abuse')}</span>
          <Icon name="external-link" size={18} />
        </button>
      </div>
    </section>

    <div class="card about-list">
      <button type="button" class="about-row" onclick={() => (bugOpen = true)}>
        <span class="about-row-label">{$t('settings_bug_report')}</span>
        <Icon name="chevron-right" size={18} />
      </button>
      <button type="button" class="about-row" onclick={() => openExternal(LICENSE_URL)}>
        <span class="about-row-label">{$t('settings_about_link_license')}</span>
        <Icon name="external-link" size={18} />
      </button>
      {#if REPO_URL}
        <button type="button" class="about-row" onclick={() => openExternal(summaryUrl)}>
          <span class="about-row-label">{$t('settings_about_link_summary')}</span>
          <Icon name="external-link" size={18} />
        </button>
        <button type="button" class="about-row" onclick={() => openExternal(thirdPartyUrl)}>
          <span class="about-row-label">{$t('settings_about_link_thirdparty')}</span>
          <Icon name="external-link" size={18} />
        </button>
        <button type="button" class="about-row" onclick={() => openExternal(REPO_URL)}>
          <span class="about-row-label">{$t('settings_about_link_repo')}</span>
          <Icon name="external-link" size={18} />
        </button>
      {/if}
      {#each aboutLinks as link (link.id)}
        <button type="button" class="about-row" onclick={() => openExternal(link.url)}>
          <span class="about-row-label">{$t(link.title as TranslationKey)}</span>
          <Icon name="external-link" size={18} />
        </button>
      {/each}
    </div>

    <div class="about-copyright">{$t('settings_about_copyright')}</div>
  </div>

  {@render moduleCards([joining('settings_group_about', false), ...blocksBetween(70)])}

  <BugReportDialog bind:open={bugOpen} />

  <div class="demo-foot" class:off={activeSection !== 'about'}>
    {#if hasDemo}
      <span class="demo-label">{$t('settings_demo_locale')}</span>
      <button type="button" class="demo-chip" class:active={demoLocale === 'en'} onclick={() => (demoLocale = 'en')}>
        {$t('settings_demo_locale_en')}
      </button>
      <button type="button" class="demo-chip" class:active={demoLocale === 'ru'} onclick={() => (demoLocale = 'ru')}>
        {$t('settings_demo_locale_ru')}
      </button>
      <button type="button" class="demo-link" disabled={demoBusy || clearBusy} onclick={askLoadDemo}>
        {demoBusy ? $t('settings_demo_loading') : $t('settings_demo_load')}
      </button>
    {/if}
    {#if hasData}
      <button type="button" class="demo-link danger" disabled={demoBusy || clearBusy} onclick={askClearData}>
        {clearBusy ? $t('settings_clear_clearing') : $t('settings_clear_data')}
      </button>
    {/if}
    <a class="demo-link" href="/settings/dev">{$t('dev_sync_title')}</a>
  </div>
  {#if dataMsg}<p class="ok-msg">{dataMsg}</p>{/if}
  {#if dataError}<div class="error-msg">{dataError}</div>{/if}
  </div>
  </div>
  </section>
</div>

{#if pendingData}
  <Dialog
    open={true}
    onclose={() => (pendingData = null)}
    title={pendingData === 'clear' ? $t('settings_clear_data') : $t('settings_demo_load')}
  >
    <div class="confirm-text">
      <p>{pendingData === 'clear' ? $t('settings_clear_confirm', { list: dataList }) : $t('settings_demo_confirm', { list: dataList })}</p>
      {#if pendingData === 'load'}
        {#each demoLosses as key (key)}<p>{$t(key as TranslationKey)}</p>{/each}
      {/if}
    </div>
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

<style>
  /* Rules written as `.settings-main :global(…)` are the card primitives the
     modules' cards (rendered inside this page) are styled with too. */
  /* Two floating panes 12 apart: the sections (280) and the chosen one. */
  .settings-layout {
    display: grid;
    grid-template-columns: 280px minmax(0, 1fr);
    gap: var(--sp-3);
    height: 100%;
    min-height: 0;
  }
  @media (max-width: 960px) {
    .settings-layout { grid-template-columns: 220px minmax(0, 1fr); }
  }
  .settings-side,
  .settings-pane {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-radius: var(--radius-pane);
    background: var(--pane);
    -webkit-backdrop-filter: var(--pane-filter);
    backdrop-filter: var(--pane-filter);
    border: 1px solid var(--pane-border);
    box-shadow: var(--pane-highlight), var(--pane-shadow);
    overflow: hidden;
  }
  .settings-side { padding: var(--sp-5) var(--sp-3) var(--sp-3); gap: var(--sp-4); }
  /* The title stays in place, as the pane's does: only the list of the sections scrolls. */
  .settings-side :global(.settings-nav) { flex: 1; min-height: 0; overflow-y: auto; }
  .side-title { padding: 0 var(--sp-2); font-size: var(--fs-xl); font-weight: var(--fw-bold); line-height: 1.4; }
  .pane-head {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    flex-shrink: 0;
    min-height: 64px;
    padding: var(--sp-4) var(--sp-6) 0;
  }
  .pane-title { font-size: var(--fs-xl); font-weight: var(--fw-bold); line-height: 1.4; }
  .pane-scroll { flex: 1; min-height: 0; overflow-y: auto; scrollbar-gutter: stable; }
  .settings-main {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
    max-width: calc(var(--page-max-narrow) + var(--sp-6) * 2);
    min-width: 0;
    padding: var(--sp-6) var(--sp-6) var(--sp-8);
  }
  .off { display: none !important; }
  /* The section's name is the pane's title: its card does not repeat it. */
  .settings-main > .card > .card-title:first-child { display: none; }
  /* A section that draws its own cards (`bare`): the hint over them, no card around. */
  .bare { display: flex; flex-direction: column; gap: var(--sp-4); min-width: 0; }
  .bare-hint { margin: 0; font-size: var(--fs-base); }

  /* local deltas over global .card: bg, internal flex layout, gap, shadow */
  .card {
    padding: var(--sp-6);
    display: flex; flex-direction: column; gap: var(--sp-4);
  }

  /* uppercase label variant — differs from global .card-title */
  .card-title {
    font-size: var(--fs-2xs); font-weight: var(--fw-bold); color: var(--text-dim);
    text-transform: uppercase; letter-spacing: 1px;
  }

  /* badges use global .badge / .badge-ok / .badge-warn */

  /* .muted uses global color; keep font-size delta */
  .settings-main :global(.muted) { font-size: var(--fs-base); }
  .settings-main :global(.small) { font-size: var(--fs-sm); }
  .settings-main :global(.ok-msg) { font-size: var(--fs-sm); color: var(--success-text); }
  /* About section */
  .card.about-card { padding: var(--sp-5); display: flex; flex-direction: column; gap: var(--sp-4); }
  .about-head { display: flex; align-items: center; gap: var(--sp-3); }
  .about-head-text { display: flex; flex-direction: column; gap: 2px; }
  .about-logo {
    display: flex; align-items: center; justify-content: center;
    width: var(--sp-12); height: var(--sp-12);
    flex-shrink: 0;
  }
  .about-logo img { width: var(--sp-12); height: var(--sp-12); object-fit: contain; }
  .about-app { font-size: var(--fs-lg); font-weight: var(--fw-extrabold); letter-spacing: -0.3px; color: var(--text); }
  .about-tagline { font-size: var(--fs-sm); color: var(--text-3); }
  /* The facts across the card, a line over and under them. */
  .about-facts { margin: 0 calc(-1 * var(--sp-5)); padding: var(--sp-1) 0; border-top: 1px solid var(--divider); border-bottom: 1px solid var(--divider); }
  .about-fact { position: relative; display: flex; align-items: baseline; gap: var(--sp-4); min-height: var(--control-h); padding: var(--sp-2) var(--sp-5); }
  .about-fact + .about-fact::before { content: ''; position: absolute; top: 0; left: 0; right: 0; height: 1px; background: var(--divider); }
  .about-key { flex: 0 0 160px; font-size: var(--fs-sm); color: var(--text-dim); }
  .about-value { flex: 1; min-width: 0; font-size: var(--fs-sm); color: var(--text-body); }
  .about-note { font-size: var(--fs-sm); color: var(--text-3); line-height: 1.5; margin: 0; }
  .about-group { display: flex; flex-direction: column; gap: var(--sp-2); }
  .about-caption { padding: 0 var(--sp-1); font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text-2); }
  /* Rows touch: the line between them is the only space (the page gives a .card a gap). */
  .card.about-list { padding: var(--sp-1); display: flex; flex-direction: column; gap: 0; }
  .about-row {
    position: relative; display: flex; align-items: center; gap: var(--sp-3); width: 100%; min-height: 48px; padding: var(--sp-2) var(--sp-4);
    border: 0; border-radius: 0; background: none; color: var(--text); font: inherit; font-size: var(--fs-base); text-align: left; cursor: pointer;
    transition: background var(--dur-fast);
  }
  /* The hover of a row: square between lines, the first and the last row take the card's corners (its radius less its padding). */
  .about-row:first-child { border-top-left-radius: calc(var(--radius-card) - var(--sp-1)); border-top-right-radius: calc(var(--radius-card) - var(--sp-1)); }
  .about-row:last-child { border-bottom-left-radius: calc(var(--radius-card) - var(--sp-1)); border-bottom-right-radius: calc(var(--radius-card) - var(--sp-1)); }
  /* A straight line between rows: a shadow would follow the rounded corners. */
  .about-row + .about-row::before { content: ''; position: absolute; top: 0; left: 0; right: 0; height: 1px; background: var(--divider); }
  .about-row:hover { background: var(--surface-row-hover); }
  .about-row :global(svg) { flex-shrink: 0; color: var(--text-dim); }
  .about-row-label { flex: 1; min-width: 0; }
  .about-copyright { font-size: var(--fs-xs); color: var(--text-3); text-align: center; }

  /* The developers' line: under a dashed line, centred, small. */
  .demo-foot {
    display: flex; flex-wrap: wrap; align-items: center; justify-content: center; gap: var(--sp-1) var(--sp-2);
    margin-top: var(--sp-2); padding-top: var(--sp-3); border-top: 1px dashed var(--divider);
  }
  .demo-label, .demo-chip, .demo-link {
    border: none; background: transparent; color: var(--text-2);
    font-size: var(--fs-sm); line-height: 1.2; padding: var(--sp-1) var(--sp-2); border-radius: var(--radius-sm);
    text-decoration: none;
  }
  .demo-label { color: var(--text-dim); }
  .demo-chip, .demo-link { cursor: pointer; }
  .demo-chip { border: 1px solid var(--border); border-radius: var(--radius-pill); }
  .demo-chip.active { color: var(--accent-text); background: var(--accent-bg); border-color: var(--accent-border); }
  .demo-chip:hover, .demo-link:hover:not(:disabled) { color: var(--text); background: var(--surface-row-hover); }
  .demo-link.danger { color: var(--danger-text); }
  .demo-link:disabled { opacity: 0.45; cursor: default; }

  .settings-main :global(.btn-sm) { padding: 0.35rem var(--sp-3); font-size: var(--fs-sm); }

  .settings-main :global(.progress-wrap) { display: flex; flex-direction: column; gap: 0.3rem; }
  .settings-main :global(.progress-bar) { height: 5px; background: var(--surface-2); border-radius: var(--radius-pill); overflow: hidden; }
  .settings-main :global(.progress-fill) { height: 100%; background: var(--accent); border-radius: var(--radius-pill); transition: width 0.2s ease; }
  .settings-main :global(.progress-label) { font-size: var(--fs-2xs); color: var(--text-2); font-family: var(--font-mono); }

  .lang-options { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: var(--sp-2); }
  .card.lang-options { padding: var(--sp-3); }

  /* Segment buttons per design: 46px, radius 11, accent tint when active */
  .lang-btn {
    display: flex; align-items: center; gap: 9px;
    min-height: var(--sp-12); padding: 0 var(--sp-4); background: var(--surface-3);
    border: 1px solid var(--border); border-radius: var(--radius-field);
    color: var(--text-body); font-size: var(--fs-base); font-weight: var(--fw-semibold); cursor: pointer;
    transition: all 0.15s; min-width: 130px;
  }
  .lang-btn:hover { border-color: var(--border-2); color: var(--text); }
  .lang-btn.active { border-color: var(--accent-border); background: var(--accent-bg); color: var(--accent-text); }

  /* The language's code as a small chip: the bundled fonts have no flag glyphs. */
  .lang-code {
    padding: 1px 6px; border: 1px solid var(--border-2); border-radius: var(--radius-xs);
    font-family: var(--font-mono); font-size: var(--fs-2xs); font-weight: var(--fw-bold);
    text-transform: uppercase; letter-spacing: 0.04em; color: var(--text-2);
  }
  .lang-btn.active .lang-code { border-color: var(--accent-border); color: var(--accent-text); }
  .confirm-text { display: flex; flex-direction: column; gap: var(--sp-3); }
  .lang-native { flex: 1; text-align: left; }
  .lang-check { display: inline-flex; color: var(--accent-text); font-weight: 700; }

  /* A row that is a switch, the whole width (developer tools), as the modules' rows. */
  .card.switch-card { padding: var(--sp-1); display: flex; flex-direction: column; gap: 0; }
  .switch-row {
    position: relative; display: flex; align-items: center; gap: var(--sp-3); width: 100%; min-height: 60px; padding: var(--sp-2) var(--sp-4);
    border: 0; border-radius: 0; background: none; color: var(--text); font: inherit; text-align: left; cursor: pointer;
    transition: background var(--dur-fast);
  }
  /* The hover of a row: square between lines, the first and the last row take the card's corners (its radius less its padding). */
  .switch-row:first-of-type { border-top-left-radius: calc(var(--radius-card) - var(--sp-1)); border-top-right-radius: calc(var(--radius-card) - var(--sp-1)); }
  .switch-row:last-of-type { border-bottom-left-radius: calc(var(--radius-card) - var(--sp-1)); border-bottom-right-radius: calc(var(--radius-card) - var(--sp-1)); }
  .switch-row:hover:not(:disabled) { background: var(--surface-row-hover); }
  .switch-row:disabled { cursor: default; opacity: 1; }
  .switch-row:disabled .toggle { opacity: 0.45; }
  /* A straight line between rows: a shadow would follow the rounded corners. */
  .switch-row + .switch-row::before { content: ''; position: absolute; top: 0; left: 0; right: 0; height: 1px; background: var(--divider); }
  .switch-row .toggle { pointer-events: none; }
  .switch-words { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: var(--sp-1); }
  .switch-title { font-size: var(--fs-body); font-weight: var(--fw-semibold); }
  .switch-sub { display: flex; align-items: center; flex-wrap: wrap; gap: var(--sp-2); font-size: var(--fs-sm); color: var(--text-2); }

  .card.update-card { padding: var(--sp-1); gap: 0; }
  .update-head { display: flex; align-items: center; gap: var(--sp-4); flex-wrap: wrap; min-height: 72px; padding: var(--sp-3) var(--sp-4); }
  .update-words { flex: 1; min-width: 200px; display: flex; flex-direction: column; gap: 2px; }
  .update-key { font-size: var(--fs-sm); color: var(--text-dim); }
  .update-version { font-size: var(--fs-lg); font-weight: var(--fw-semibold); color: var(--text); }
  .update-line { font-size: var(--fs-sm); font-weight: var(--fw-semibold); }
  .update-line.ok { color: var(--success-text); }
  .update-line.new { color: var(--accent-text); }
  .update-acts { display: flex; gap: var(--sp-2); flex-wrap: wrap; }
  .update-work { padding: var(--sp-3) var(--sp-4) var(--sp-4); border-top: 1px solid var(--divider); }
  .update-work .muted { margin: 0; }
  .update-note, .update-fail {
    display: flex; align-items: flex-start; gap: var(--sp-3); padding: var(--sp-3) var(--sp-4);
    border-radius: var(--radius-md); font-size: var(--fs-sm); line-height: 1.5;
  }
  .update-note { background: var(--surface-2); color: var(--text-2); }
  .update-fail { background: var(--danger-bg); color: var(--text); }
  .update-note :global(svg), .update-fail :global(svg) { flex-shrink: 0; margin-top: 1px; }
  .update-fail :global(svg) { color: var(--danger-text); }
  .update-note-words { display: flex; flex-direction: column; gap: var(--sp-2); align-items: flex-start; }

  .card.theme-card { padding: var(--sp-3); }
  .theme-options { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--sp-2); }

  .theme-opt {
    display: flex; align-items: center; gap: var(--sp-3);
    min-height: 56px; padding: 0 var(--sp-4); background: var(--surface-3);
    border: 1px solid var(--border); border-radius: var(--radius-field);
    color: var(--text-body); font-size: var(--fs-md); font-weight: var(--fw-semibold); cursor: pointer;
    transition: all 0.15s;
  }
  .theme-opt:hover:not(:disabled) { border-color: var(--border-2); color: var(--text); }
  .theme-opt.active { border-color: var(--accent-border); background: var(--accent-bg); color: var(--accent-text); }
  .theme-opt:disabled { opacity: 0.4; cursor: not-allowed; }
  /* The theme's icon on a tile of its own; the chosen one's is the accent's. */
  .theme-icon {
    flex: none; display: grid; place-items: center; width: var(--control-h-sm); height: var(--control-h-sm);
    border-radius: var(--radius-sm); background: var(--surface-2); color: var(--text-2);
  }
  .theme-opt.active .theme-icon { background: transparent; color: var(--accent-text); }
  .theme-name { flex: 1; text-align: left; }
  .theme-customize {
    margin-top: var(--sp-3);
    padding: var(--sp-3) var(--sp-2) 0;
    border-top: 1px solid var(--border);
  }

  .settings-main :global(.dir-input) {
    flex: 1; height: 44px; background: var(--surface-3); border: 1px solid var(--border);
    border-radius: var(--radius); padding: 0 var(--sp-3);
    font-size: var(--fs-sm); color: var(--text-body); font-family: var(--font-mono);
    outline: none; text-overflow: ellipsis;
  }
  .settings-main :global(.dir-input:focus) { border-color: var(--accent-border); }
  .settings-main :global(.error-msg) { font-size: var(--fs-sm); color: var(--danger-text); }

  /* Form fields of the cards */
  .settings-main :global(.field) { display: flex; flex-direction: column; gap: var(--sp-2); }
  .settings-main :global(.field-label) { font-size: var(--fs-sm); color: var(--text); font-weight: var(--fw-semibold); }
  .settings-main :global(.field .dir-input) { width: 100%; }

  /* Standard form input (password / number / time) — matches CustomSelect box */
  .settings-main :global(.field-input) {
    width: 100%; height: var(--control-h-lg);
    background: var(--surface-3); border: 1px solid var(--border);
    border-radius: var(--radius-field); padding: 0 var(--sp-3);
    font-size: var(--fs-base); color: var(--text); font-family: inherit;
    outline: none; transition: border-color var(--dur-fast), box-shadow var(--dur-fast);
  }
  .settings-main :global(.field-input:focus) { border-color: var(--accent-border); box-shadow: 0 0 0 3px var(--accent-bg); }

</style>
