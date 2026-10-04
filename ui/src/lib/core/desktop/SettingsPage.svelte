<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import { locale, t, type TranslationKey } from '$lib/core/i18n';
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
  import type { Locale } from '$lib/core/i18n';
  import { formatError } from '$lib/core/utils';
  import { updaterStore } from '$lib/core/store/updater.svelte';
  import Dialog from '$lib/core/ui/Dialog.svelte';
  import LockSetupForm from '$lib/core/lock/LockSetupForm.svelte';
  import SyncSettings from '$lib/core/sync/SyncSettings.svelte';
  import BugReportDialog from '$lib/core/BugReportDialog.svelte';
  import HotkeySettings from '$lib/core/desktop/HotkeySettings.svelte';
  import ModulesSettings from '$lib/core/ModulesSettings.svelte';
  import SettingsNav, { type SettingsNavGroup } from '$lib/core/desktop/SettingsNav.svelte';
  import { formatCommand, keybindingOverrides } from '$lib/core/keybindings';
  import { registry } from '$lib/core/registry';
  import { product } from '$lib/core/product';
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
        const item = (sec: SettingsSection) => ({ id: sec.id, label: $t(sec.title as TranslationKey) });
        return {
          label: $t(block.group as TranslationKey),
          items: core
            ? [
                ...joining(core.group, true).sections.map(item),
                ...core.ids.map((id) => ({ id, label: coreLabel(id) })),
                ...joining(core.group, false).sections.map(item),
              ]
            : block.sections.map(item),
        };
      }),
  );
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
  let sectionsEl = $state<HTMLElement | null>(null);
  let layoutEl = $state<HTMLElement | null>(null);
  // Set on nav click; scroll-spy stays quiet until the smooth scroll settles
  let navScrolling = false;

  function scrollToSection(id: string) {
    activeSection = id;
    navScrolling = true;
    document.getElementById(id)?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }

  function scrollParent(el: HTMLElement): HTMLElement {
    let p = el.parentElement;
    while (p && !/auto|scroll/.test(getComputedStyle(p).overflowY)) p = p.parentElement;
    return p ?? document.documentElement;
  }

  // Scroll-spy: active = last section above the 25% line, or the last one at the bottom
  function observeSections(): () => void {
    if (!sectionsEl) return () => {};
    const sections = [...sectionsEl.querySelectorAll<HTMLElement>(':scope > [id]')];
    const scroller = scrollParent(sectionsEl);
    let settleTimer: ReturnType<typeof setTimeout> | undefined;

    // The nav is as tall as the scroller lets it be (SettingsNav.svelte).
    const fit = () => layoutEl?.style.setProperty('--settings-view-h', `${scroller.clientHeight}px`);
    const sizes = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(fit);
    sizes?.observe(scroller);
    fit();

    const update = () => {
      const line = scroller.getBoundingClientRect().top + scroller.clientHeight * 0.25;
      const atBottom = scroller.scrollTop + scroller.clientHeight >= scroller.scrollHeight - 2;
      const current = atBottom
        ? sections.at(-1)
        : sections.findLast((s) => s.getBoundingClientRect().top <= line) ?? sections[0];
      if (current) activeSection = current.id;
    };
    const onScroll = () => {
      if (!navScrolling) return update();
      clearTimeout(settleTimer);
      settleTimer = setTimeout(() => (navScrolling = false), 150);
    };

    update();
    scroller.addEventListener('scroll', onScroll, { passive: true });
    return () => {
      clearTimeout(settleTimer);
      sizes?.disconnect();
      scroller.removeEventListener('scroll', onScroll);
    };
  }

  const THEME_PRESETS = {
    chrome: {
      dark: ['#0b0b10', '#14141c', '#1c1c26', '#0e1620', '#16120e', '#1a1224'],
      light: ['#fafafd', '#ffffff', '#f0eef8', '#ebe8f2', '#f4f1ea', '#efe8f0'],
    },
    accent: ['#8b7bff', '#6d5cf0', '#60a5fa', '#2dd4bf', '#f472b6', '#34d399'],
    bg: {
      dark: ['#08080c', '#0c0c12', '#101018', '#0a1016', '#12100c', '#100c16'],
      light: ['#f4f3f8', '#ffffff', '#eeeef4', '#f6f4ee', '#f2eef8', '#eaeaf0'],
    },
  } as const;

  const languages: { value: Locale; label: string; native: string }[] = [
    { value: 'en', label: 'English', native: 'English' },
    { value: 'ru', label: 'Russian', native: 'Русский' },
  ];

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
  const hasDemo = demoModules.length > 0;
  /** The sections whose data the demo set replaces and "Clear app data" deletes. */
  const dataList = $derived(demoModules.map((m) => $t(m.title as TranslationKey)).join(', '));
  let demoLocale = $state<'en' | 'ru'>(get(locale));
  // What goes with the lock's key when the demo data replaces it: each module's own sentence.
  const demoLosses = $derived(registry.active.flatMap((m) => (m.vaultResetNote ? [m.vaultResetNote] : [])));
  let demoBusy = $state(false);
  let pendingData = $state<'load' | 'clear' | null>(null);
  let clearBusy = $state(false);
  let dataMsg = $state('');
  let dataError = $state('');

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
    unlisteners.push(observeSections());
    // A link to one card (`/settings#updates`, the palette's "Check for updates").
    const wanted = location.hash.slice(1);
    if (wanted && document.getElementById(wanted)) scrollToSection(wanted);

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
    pendingData = 'load';
  }

  function askClearData() {
    if (demoBusy || clearBusy) return;
    if (!isTauri) {
      dataError = formatError('Not running in app');
      return;
    }
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
    if (!isTauri) {
      dataError = formatError('Not running in app');
      return;
    }
    demoBusy = true;
    dataMsg = '';
    dataError = '';
    try {
      await api.demo.seed(demoLocale);
      await refreshCatalogStores();
      dataMsg = $t('settings_demo_done');
      setTimeout(() => (dataMsg = ''), 4000);
    } catch (e) {
      dataError = formatError(e);
    } finally {
      demoBusy = false;
    }
  }

  async function clearAppData() {
    if (demoBusy || clearBusy) return;
    if (!isTauri) {
      dataError = formatError('Not running in app');
      return;
    }
    clearBusy = true;
    dataMsg = '';
    dataError = '';
    try {
      await api.app.clearData();
      await refreshCatalogStores();
      dataMsg = $t('settings_clear_done');
      setTimeout(() => (dataMsg = ''), 4000);
    } catch (e) {
      dataError = formatError(e);
    } finally {
      clearBusy = false;
    }
  }

</script>

{#snippet moduleCards(blocks: Block[])}
  {#each blocks as block (block.group)}
    {#each block.sections as section (section.id)}
      <div class="card" id={section.id}>
        <div class="card-title">
          {$t(section.title as TranslationKey)}
          {#if section.badge}<span class="badge badge-warn">{$t(section.badge as TranslationKey)}</span>{/if}
        </div>
        {#if section.hint}<p class="muted">{$t(section.hint as TranslationKey)}</p>{/if}
        <section.component />
      </div>
    {/each}
  {/each}
{/snippet}

<div class="page settings-layout" bind:this={layoutEl}>
  <SettingsNav groups={navGroups} active={activeSection} onselect={scrollToSection} />

  <div class="settings-main" bind:this={sectionsEl}>
  <h1>{$t('settings_title')}</h1>

  <!-- The modules' cards before the general block: the browser's Camoufox. -->
  {@render moduleCards(blocksBetween(-Infinity, 20))}

  {@render moduleCards([joining('settings_group_general', true)])}

  {#if withModules}
    <!-- Modules -->
    <div class="card" id="modules">
      <div class="card-title">{$t('modules_section')}</div>
      <p class="muted">{$t('modules_hint')}</p>
      <ModulesSettings />
    </div>
  {/if}

  <!-- Language -->
  <div class="card" id="language">
    <div class="card-title">{$t('settings_section_language')}</div>
    <p class="muted">{$t('settings_language_label')}</p>
    <div class="lang-options">
      {#each languages as lang}
        <button
          class="lang-btn"
          class:active={$locale === lang.value}
          onclick={() => locale.set(lang.value)}
        >
          <span class="lang-code">{lang.value}</span>
          <span class="lang-native">{lang.native}</span>
          {#if $locale === lang.value}<span class="lang-check">✓</span>{/if}
        </button>
      {/each}
    </div>
  </div>

  <!-- Theme -->
  <div class="card" id="theme">
    <div class="card-title">{$t('settings_section_theme')}</div>
    <div class="theme-options">
      <button class="theme-opt" class:active={$theme === 'dark'} onclick={() => theme.set('dark')}>
        <span class="theme-icon"><Icon name="moon" size={15} /></span>
        <span>{$t('settings_theme_dark')}</span>
        {#if $theme === 'dark'}<span class="lang-check">✓</span>{/if}
      </button>
      <button class="theme-opt" class:active={$theme === 'light'} onclick={() => theme.set('light')}>
        <span class="theme-icon"><Icon name="sun" size={15} /></span>
        <span>{$t('settings_theme_light')}</span>
        {#if $theme === 'light'}<span class="lang-check">✓</span>{/if}
      </button>
    </div>
    <div class="theme-customize">
      <ColorField
        label={$t('settings_theme_chrome')}
        value={resolvedOverride($themeCustom, $theme, 'chrome')}
        presets={[...THEME_PRESETS.chrome[$theme]]}
        onchange={(hex) => setThemeOverride('chrome', hex)}
      />
      <ColorField
        label={$t('settings_theme_accent')}
        value={resolvedOverride($themeCustom, $theme, 'accent')}
        presets={[...THEME_PRESETS.accent]}
        onchange={(hex) => setThemeOverride('accent', hex)}
      />
      <ColorField
        label={$t('settings_theme_bg')}
        value={resolvedOverride($themeCustom, $theme, 'bg')}
        presets={[...THEME_PRESETS.bg[$theme]]}
        onchange={(hex) => setThemeOverride('bg', hex)}
      />
      <button
        type="button"
        class="btn btn-ghost btn-sm theme-reset"
        disabled={!hasThemeOverrides($themeCustom, $theme)}
        onclick={resetThemeOverrides}
      >
        {$t('settings_theme_reset')}
      </button>
    </div>
  </div>

  <!-- Developer Tools -->
  <div class="card" id="devtools">
    <div class="card-title">{$t('inspector_developer_tools')}</div>
    <div class="dev-tools-row">
      <div class="dev-tools-info">
        <span>{$t('inspector_toggle')}</span>
        <span class="muted">{$t('inspector_hotkey_hint', { keys: formatCommand('app.inspector', $keybindingOverrides) || $t('hotkey_unbound') })}</span>
      </div>
      <button
        class="toggle"
        class:on={inspectorApp.enabled}
        onclick={() => inspectorApp.toggle()}
        aria-pressed={inspectorApp.enabled}
        aria-label={$t('inspector_toggle')}
      ></button>
    </div>
  </div>

  <div id="hotkeys">
    <HotkeySettings />
  </div>

  <!-- System tray -->
  <div class="card" id="tray">
    <div class="card-title">{$t('settings_tray_section')}</div>
    <div class="dev-tools-row">
      <div class="dev-tools-info">
        <span>{$t('settings_tray_minimize')}</span>
        <span class="muted">{$t('settings_tray_minimize_hint')}</span>
      </div>
      <button
        class="toggle"
        class:on={trayMinimize}
        disabled={!isTauri}
        onclick={() => { trayMinimize = !trayMinimize; saveTray(); }}
        aria-pressed={trayMinimize}
        aria-label={$t('settings_tray_minimize')}
      ></button>
    </div>
    <div class="dev-tools-row">
      <div class="dev-tools-info">
        <span>{$t('settings_tray_close')}</span>
        <span class="muted">{$t('settings_tray_close_hint')}</span>
      </div>
      <button
        class="toggle"
        class:on={trayClose}
        disabled={!isTauri}
        onclick={() => { trayClose = !trayClose; saveTray(); }}
        aria-pressed={trayClose}
        aria-label={$t('settings_tray_close')}
      ></button>
    </div>
    <div class="dev-tools-row">
      <div class="dev-tools-info">
        <span>{$t('settings_tray_start_hidden')}</span>
        <span class="muted">{$t('settings_tray_start_hidden_hint')}</span>
      </div>
      <button
        class="toggle"
        class:on={trayStartHidden}
        disabled={!isTauri}
        onclick={() => { trayStartHidden = !trayStartHidden; saveTray(); }}
        aria-pressed={trayStartHidden}
        aria-label={$t('settings_tray_start_hidden')}
      ></button>
    </div>
  </div>

  {@render moduleCards([joining('settings_group_general', false), ...blocksBetween(20, 40)])}

  {@render moduleCards([joining('settings_group_security', true)])}

  <!-- App lock -->
  <div class="card" id="lock">
    <div class="card-title">{$t('settings_lock_section')} <span class="badge badge-warn">{$t('settings_sync_beta')}</span></div>
    <p class="muted">{$t('settings_lock_hint')}</p>
    <LockSetupForm />
  </div>

  {@render moduleCards([joining('settings_group_security', false), ...blocksBetween(40, 50)])}

  <!-- The backup service's card -->
  {@render moduleCards([joining('settings_group_data', true)])}

  <!-- Sync (beta) -->
  <div class="card" id="sync">
    <div class="card-title">{$t('settings_sync_section')} <span class="badge badge-warn">{$t('settings_sync_beta')}</span></div>
    <p class="muted">{$t('settings_sync_hint')}</p>
    <SyncSettings />
  </div>

  {@render moduleCards([joining('settings_group_data', false), ...blocksBetween(50, 70)])}

  {@render moduleCards([joining('settings_group_about', true)])}

  <!-- App updates -->
  <div class="card" id="updates">
    <div class="card-title">{$t('settings_update_section')}</div>

    <div class="version-table">
      <div class="version-row">
        <span class="version-label">{$t('settings_about_version')}</span>
        <span class="version-value">{appVersion ? `v${appVersion}` : '—'}</span>
      </div>
    </div>

    <div class="btn-row">
      {#if updaterStore.status === 'available' && updaterStore.supported}
        <button class="btn btn-primary btn-sm" onclick={() => updaterStore.install()}>
          {$t('settings_update_install')}
        </button>
      {/if}
      <button
        class="btn btn-ghost btn-sm"
        disabled={updaterStore.status === 'checking' ||
          updaterStore.status === 'downloading' ||
          updaterStore.status === 'installing'}
        onclick={() => updaterStore.check(false)}
      >
        {updaterStore.status === 'checking'
          ? $t('settings_update_checking')
          : $t('settings_update_check')}
      </button>
    </div>

    {#if updaterStore.status === 'upToDate'}
      <p class="ok-msg">{$t('settings_update_up_to_date')}</p>
    {:else if updaterStore.status === 'available'}
      <p class="warn-msg">{$t('settings_update_available', { version: updaterStore.version })}</p>
      {#if !updaterStore.supported}
        <p class="muted small">{$t('settings_update_unsupported')}</p>
        <div class="btn-row">
          <button class="btn btn-ghost btn-sm" onclick={() => openExternal(RELEASES_URL)}>
            <Icon name="external-link" size={14} />
            {$t('settings_update_open_releases')}
          </button>
        </div>
      {/if}
    {:else if updaterStore.status === 'downloading'}
      <div class="progress-wrap">
        <div class="progress-bar">
          <div class="progress-fill" style="width: {updaterStore.progress}%"></div>
        </div>
        <span class="progress-label">
          {$t('settings_update_downloading')} · {updaterStore.progress}%
        </span>
      </div>
    {:else if updaterStore.status === 'installing'}
      <p class="muted small">{$t('settings_update_installing')}</p>
    {:else if updaterStore.status === 'error'}
      <!-- The plugin's own text is in the console (store/updater.svelte.ts). -->
      <div class="error-msg">{$t('settings_update_failed')}</div>
    {/if}
  </div>

  <!-- About -->
  <div class="card" id="about">
    <div class="card-title">{$t('settings_section_about')}</div>

    <div class="about-head">
      <span class="about-logo">
        <img src="/logo.png" alt="" />
      </span>
      <div class="about-head-text">
        <div class="about-app">{product.name}</div>
        <div class="about-tagline">{product.tagline[$locale]}</div>
      </div>
    </div>

    <div class="version-table">
      <div class="version-row">
        <span class="version-label">{$t('settings_about_version')}</span>
        <span class="version-value">{appVersion ? `v${appVersion}` : '—'}</span>
      </div>
      <div class="version-row">
        <span class="version-label">{$t('settings_about_license')}</span>
        <span class="version-value">PolyForm Perimeter 1.0.1</span>
      </div>
    </div>

    <p class="about-note">{$t('settings_about_license_note')}</p>

    <div class="about-links">
      <button type="button" class="link-btn" onclick={() => (bugOpen = true)}>
        <Icon name="info" size={14} />
        <span>{$t('settings_bug_report')}</span>
      </button>
      <button type="button" class="link-btn" onclick={() => openExternal(LICENSE_URL)}>
        <Icon name="external-link" size={14} />
        <span>{$t('settings_about_link_license')}</span>
      </button>
      {#if REPO_URL}
        <button type="button" class="link-btn" onclick={() => openExternal(summaryUrl)}>
          <Icon name="external-link" size={14} />
          <span>{$t('settings_about_link_summary')}</span>
        </button>
        <button type="button" class="link-btn" onclick={() => openExternal(thirdPartyUrl)}>
          <Icon name="external-link" size={14} />
          <span>{$t('settings_about_link_thirdparty')}</span>
        </button>
        <button type="button" class="link-btn" onclick={() => openExternal(REPO_URL)}>
          <Icon name="external-link" size={14} />
          <span>{$t('settings_about_link_repo')}</span>
        </button>
      {/if}
      {#each aboutLinks as link (link.id)}
        <button type="button" class="link-btn" onclick={() => openExternal(link.url)}>
          <Icon name="external-link" size={14} />
          <span>{$t(link.title as TranslationKey)}</span>
        </button>
      {/each}
    </div>

    <div class="about-copyright">{$t('settings_about_copyright')}</div>
  </div>

  {@render moduleCards([joining('settings_group_about', false), ...blocksBetween(70)])}

  <BugReportDialog bind:open={bugOpen} />

  <div class="demo-foot">
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

<style>
  /* Rules written as `.settings-main :global(…)` are the card primitives the
     modules' cards (rendered inside this page) are styled with too. */
  /* Two columns: sticky nav + content rail of --page-max-narrow width */
  .settings-layout {
    --nav-w: 200px;
    --page-max: calc(var(--page-max-narrow) + var(--nav-w) + var(--sp-8));
    display: grid;
    grid-template-columns: var(--nav-w) minmax(0, 1fr);
    column-gap: var(--sp-8);
    align-items: start;
  }
  .settings-main { display: flex; flex-direction: column; gap: var(--sp-3); min-width: 0; }
  .settings-main > [id] { scroll-margin-top: var(--sp-3); }

  @media (max-width: 960px) {
    .settings-layout { --page-max: var(--page-max-narrow); grid-template-columns: minmax(0, 1fr); }
    .settings-layout > :global(.settings-nav) { display: none; }
  }

  /* bare h1 (not inside .page-header) — global .page-header h1 doesn't apply */
  h1 { font-size: var(--fs-2xl); font-weight: var(--fw-extrabold); letter-spacing: -0.6px; margin-bottom: var(--sp-2); }

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

  .settings-main :global(.status-row) { display: flex; align-items: center; gap: var(--sp-3); }

  /* badges use global .badge / .badge-ok / .badge-warn */

  /* .muted uses global color; keep font-size delta */
  .settings-main :global(.muted) { font-size: var(--fs-base); }
  .settings-main :global(.small) { font-size: var(--fs-sm); }
  .link-btn {
    align-self: flex-start; background: none; border: none; padding: 0; cursor: pointer;
    font-size: var(--fs-xs); color: var(--text-faint); text-decoration: underline;
  }
  .link-btn:hover { color: var(--text); }
  .settings-main :global(.ok-msg) { font-size: var(--fs-sm); color: var(--success-text); }
  .settings-main :global(.warn-msg) { font-size: var(--fs-sm); color: var(--warn-text); }

  .settings-main :global(.version-table) { display: flex; flex-direction: column; gap: 0.35rem; }
  .settings-main :global(.version-row) { display: flex; align-items: baseline; gap: var(--sp-2); }
  .settings-main :global(.version-label) { font-size: 0.82rem; color: var(--text-faint); min-width: 120px; flex-shrink: 0; }
  .settings-main :global(.version-value) { font-size: var(--fs-sm); color: var(--text-body); font-family: var(--font-mono); }
  .settings-main :global(.path-value) { font-size: var(--fs-2xs); word-break: break-all; }

  /* About section */
  .about-head { display: flex; align-items: center; gap: 13px; }
  .about-head-text { display: flex; flex-direction: column; gap: 2px; }
  .about-logo {
    display: flex; align-items: center; justify-content: center;
    width: 44px; height: 44px;
    flex-shrink: 0;
  }
  .about-logo img { width: 44px; height: 44px; object-fit: contain; }
  .about-app { font-size: var(--fs-lg); font-weight: var(--fw-extrabold); letter-spacing: -0.3px; color: var(--text); }
  .about-tagline { font-size: var(--fs-sm); color: var(--text-3); }
  .about-note { font-size: var(--fs-sm); color: var(--text-3); line-height: 1.5; margin: 0; }
  .about-links { display: flex; flex-wrap: wrap; gap: var(--sp-2) var(--sp-4); }
  .link-btn {
    display: inline-flex; align-items: center; gap: 0.4rem;
    background: none; border: none; padding: 0; cursor: pointer;
    color: var(--accent-text-3); font-size: var(--fs-sm); font-weight: 500;
  }
  .link-btn:hover { color: var(--accent-text); }
  .link-btn:hover { text-decoration: underline; }
  .about-copyright { font-size: var(--fs-xs); color: var(--text-3); }

  .demo-foot {
    display: flex; flex-wrap: wrap; align-items: center; gap: 0.15rem 0.45rem;
    margin-top: var(--sp-2);
  }
  .demo-label, .demo-chip, .demo-link {
    border: none; background: transparent; color: var(--text-faint);
    font-size: 0.72rem; line-height: 1.2; padding: 0.1rem 0.2rem;
    text-decoration: none;
  }
  .demo-chip, .demo-link { cursor: pointer; }
  .demo-chip.active { color: var(--text-body); }
  .demo-chip:hover, .demo-link:hover:not(:disabled) { color: var(--text); text-decoration: underline; }
  .demo-link.danger:hover:not(:disabled) { color: var(--danger-text); }
  .demo-link:disabled { opacity: 0.45; cursor: default; }

  .settings-main :global(.btn-sm) { padding: 0.35rem var(--sp-3); font-size: var(--fs-sm); }
  .settings-main :global(.btn-row) { display: flex; gap: var(--sp-2); align-items: center; }

  .settings-main :global(.progress-wrap) { display: flex; flex-direction: column; gap: 0.3rem; }
  .settings-main :global(.progress-bar) { height: 5px; background: var(--surface-2); border-radius: 999px; overflow: hidden; }
  .settings-main :global(.progress-fill) { height: 100%; background: var(--accent); border-radius: 999px; transition: width 0.2s ease; }
  .settings-main :global(.progress-label) { font-size: var(--fs-2xs); color: var(--text-2); font-family: var(--font-mono); }

  .lang-options { display: flex; gap: 0.625rem; }

  /* Segment buttons per design: 46px, radius 11, accent tint when active */
  .lang-btn {
    display: flex; align-items: center; gap: 9px;
    height: var(--control-h-lg); padding: 0 20px; background: var(--surface-3);
    border: 1px solid var(--border); border-radius: var(--radius-field);
    color: var(--text-body); font-size: 0.9rem; font-weight: var(--fw-semibold); cursor: pointer;
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
  .lang-check { color: var(--accent-text); font-weight: 700; }

  .settings-main :global(.dev-tools-row) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-4);
  }

  /* Separate stacked toggle rows within one card (e.g. the tray section) */
  .settings-main :global(.dev-tools-row + .dev-tools-row) {
    margin-top: var(--sp-4);
    padding-top: var(--sp-4);
    border-top: 1px solid var(--border);
  }

  .settings-main :global(.dev-tools-info) {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: var(--fs-sm);
    color: var(--text);
  }

  .theme-options { display: flex; gap: 0.625rem; }

  .theme-opt {
    display: flex; align-items: center; gap: 9px;
    height: var(--control-h-lg); padding: 0 20px; background: var(--surface-3);
    border: 1px solid var(--border); border-radius: var(--radius-field);
    color: var(--text-body); font-size: 0.9rem; font-weight: var(--fw-semibold); cursor: pointer;
    transition: all 0.15s; min-width: 110px;
  }
  .theme-opt:hover:not(:disabled) { border-color: var(--border-2); color: var(--text); }
  .theme-opt.active { border-color: var(--accent-border); background: var(--accent-bg); color: var(--accent-text); }
  .theme-opt:disabled { opacity: 0.4; cursor: not-allowed; }
  .theme-icon { font-size: var(--fs-md); display: flex; }
  .theme-customize {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
    margin-top: var(--sp-4);
    padding-top: var(--sp-4);
    border-top: 1px solid var(--border);
  }
  .theme-reset { align-self: flex-start; }

  .settings-main :global(.dir-row) {
    display: flex; align-items: center; gap: 0.4rem;
  }
  .settings-main :global(.dir-input) {
    flex: 1; height: 44px; background: var(--surface-3); border: 1px solid var(--border);
    border-radius: var(--radius); padding: 0 var(--sp-3);
    font-size: 0.82rem; color: var(--text-body); font-family: var(--font-mono);
    outline: none; text-overflow: ellipsis;
  }
  .settings-main :global(.dir-input:focus) { border-color: var(--accent-border); }
  .settings-main :global(.btn-icon) { width: 44px; height: 44px; justify-content: center; padding: 0; }
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
