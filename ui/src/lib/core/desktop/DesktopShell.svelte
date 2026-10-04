<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import '@fontsource-variable/manrope/index.css';
  import '@fontsource-variable/jetbrains-mono/index.css';
  import '$lib/core/styles/tokens.css';
  import '$lib/core/styles/base.css';
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { onMount, untrack } from 'svelte';
  import { get } from 'svelte/store';
  import type { Snippet } from 'svelte';
  import { t, locale, type TranslationKey } from '$lib/core/i18n';
  import type { TrayLabels } from '$lib/core/api';
  import type { ToolItem } from '$lib/core/module';
  import { theme, toggleTheme } from '$lib/core/theme';
  import Icon from '$lib/core/Icon.svelte';
  import { api, isSecondaryWindow, windowLabel } from '$lib/core/api';
  import CommandPalette from '$lib/core/desktop/CommandPalette.svelte';
  import { commands, registerDirectoryCommands } from '$lib/core/commands';
  import { leavesOff, registry } from '$lib/core/registry';
  import ModulesPicker from '$lib/core/ModulesPicker.svelte';
  import ConfirmHost from '$lib/core/ui/ConfirmHost.svelte';
  import { appLock } from '$lib/core/lock/store.svelte';
  import AppLockGate from '$lib/core/lock/AppLockGate.svelte';
  import { updaterStore } from '$lib/core/store/updater.svelte';
  import UpdateBanner from '$lib/core/desktop/UpdateBanner.svelte';
  import { listen } from '@tauri-apps/api/event';
  import { syncStore } from '$lib/core/store/sync.svelte';
  import UIInspector from '$lib/core/inspector/UIInspector.svelte';
  import { inspectorApp } from '$lib/core/inspector/inspector.svelte';
  import { matches, stockHistoryChord } from '$lib/core/keybindings';
  import { product } from '$lib/core/product';
  import WindowFrame from '$lib/core/desktop/WindowFrame.svelte';
  import { isCsd } from '$lib/core/desktop/csd';
  import { registerShellCommands } from '$lib/core/desktop/shell-commands';

  let { children }: { children: Snippet } = $props();

  // What the product's modules put into the shell (platform-spec 11.7): the
  // navigation, the top bar's buttons, the overlays. The registry is loaded
  // by the root layout before the shell mounts; a module the user switches
  // off leaves them at once (section 12).
  const nav = $derived(registry.nav());
  const tools = $derived(registry.tools());
  const titleTools = $derived(tools.filter((tool) => tool.place === 'title'));
  const barTools = $derived(tools.filter((tool) => tool.place !== 'title'));
  const bottomOverlays = $derived(registry.overlays('bottom'));
  const contentOverlays = $derived(registry.overlays('content'));
  const windowOverlays = $derived(registry.overlays('window'));

  const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
  // A module's own window (the notes popout, quick capture): theme and chrome only.
  const standaloneNotes = isSecondaryWindow();
  const windowTitle = standaloneNotes ? registry.windowTitle(windowLabel()) : undefined;

  // A product of one module with one screen (Notes, Pass, Chat) has nothing
  // to navigate between: no navigation row and no second bar with the name
  // the title bar already shows. With client-side decorations (Linux) the
  // title bar carries the tools, the theme toggle and Settings; on Windows
  // and macOS, whose title bar is the system's, one slim bar does.
  const only = registry.only;
  const single = $derived(only !== undefined && nav.length <= 1);
  /** The screen is outside the module (Settings): the chrome shows the way back. */
  const away = $derived(single && $page.url.pathname.startsWith('/settings'));
  const homeTitle = $derived(only ? $t(only.title as TranslationKey) : '');

  /** Settings is a place to return from: its button closes it when it is open. */
  function toggleSettings() {
    void goto(isActive('/settings') ? '/' : '/settings');
  }

  // Build the tray menu strings for the active locale: the shell's own and the
  // modules' entries. Parametrized entries ({n}) are passed as raw templates —
  // the backend fills in the count.
  function buildTrayLabels(): TrayLabels {
    const tt = get(t);
    return {
      show: tt('tray_show'),
      hide: tt('tray_hide'),
      quit: tt('tray_quit'),
      tooltip: tt('tray_tooltip'),
      ...registry.trayLabels(),
    };
  }

  /** Fresh webview profile (new install or app rename): take the language saved in the database. */
  async function restoreLocale() {
    if (!isTauri || localStorage.getItem('vb_locale')) return;
    try {
      const saved = await api.settings.getLocale();
      if ((saved === 'en' || saved === 'ru') && saved !== get(locale)) locale.set(saved);
    } catch {}
  }

  // Tray menu and browser extension follow the app language
  function syncTrayLabels() {
    if (!isTauri) return;
    api.settings.setTrayLabels(buildTrayLabels()).catch(() => {});
    api.settings.setLocale(get(locale)).catch(() => {});
  }

  /** Plain text fields use the browser undo stack; the rich editor handles its own. */
  function isNativeTextField(target: EventTarget | null): boolean {
    if (target instanceof HTMLTextAreaElement) return !target.readOnly && !target.disabled;
    if (!(target instanceof HTMLInputElement) || target.readOnly || target.disabled) return false;
    const skip = ['button', 'checkbox', 'radio', 'file', 'range', 'color', 'submit', 'reset', 'image', 'hidden'];
    return !skip.includes(target.type);
  }

  function handleKeyBack(e: KeyboardEvent) {
    if (!e.defaultPrevented && isNativeTextField(e.target)) {
      if (matches(e, 'edit.undo')) {
        e.preventDefault();
        document.execCommand('undo');
        return;
      }
      if (matches(e, 'edit.redo')) {
        e.preventDefault();
        document.execCommand('redo');
        return;
      }
      if (stockHistoryChord(e)) {
        e.preventDefault();
        return;
      }
    }
    if (matches(e, 'app.inspector')) {
      e.preventDefault();
      inspectorApp.toggle();
      return;
    }
    if (e.altKey && e.key === 'ArrowLeft') {
      e.preventDefault();
      window.history.back();
    }
  }

  function handleMouseBack(e: MouseEvent) {
    if (e.button === 3) {
      e.preventDefault();
      window.history.back();
    }
  }

  // The page of a module the user switched off: the shell goes to `/`.
  $effect(() => {
    if (!standaloneNotes && leavesOff($page.url.pathname)) void goto('/', { replaceState: true });
  });

  onMount(() => {
    registerShellCommands();
    registerDirectoryCommands();
    // Every module's commands: the palette lists those of the modules that are on.
    for (const m of registry.modules) {
      commands.as(m.id, () => {
        m.commands?.();
        if (m.palette) commands.addProvider(m.palette);
      });
    }
    const unsub = theme.subscribe((val) => {
      document.body.dataset.theme = val;
    });

    // Notes popout: only theme + window chrome. Main window owns tray / dock / updater,
    // and starts the modules (their stores, subscriptions and tray entries).
    if (!standaloneNotes) {
      void appLock.listen();
      void registry.startAll();
    }

    // Native back button support for WebKitGTK (Tauri on Linux)
    window.addEventListener('keydown', handleKeyBack);
    window.addEventListener('mouseup', handleMouseBack);

    const unlistenSyncData = standaloneNotes ? Promise.resolve(() => {}) : syncStore.listenDataChanged();

    const unsubLocale = standaloneNotes
      ? () => {}
      : locale.subscribe(() => syncTrayLabels());
    if (!standaloneNotes) void restoreLocale().then(syncTrayLabels);

    // The tray's own entries; a module's entries (profiles, the generator) are listened for by the module.
    // `listen` needs the Tauri bridge: the plain browser (`vite dev`) has none.
    const trayUnlisteners = standaloneNotes || !isTauri ? [] : [listen<string>('tray://navigate', (e) => goto(e.payload))];

    let updateTimer: ReturnType<typeof setTimeout> | undefined;
    if (!standaloneNotes) {
      updaterStore.init();
      updateTimer = setTimeout(() => updaterStore.check(true), 5000);
    }

    return () => {
      unsub();
      if (updateTimer) clearTimeout(updateTimer);
      window.removeEventListener('keydown', handleKeyBack);
      window.removeEventListener('mouseup', handleMouseBack);
      unlistenSyncData.then((fn) => fn());
      unsubLocale();
      trayUnlisteners.forEach((p) => p.then((fn) => fn()));
      if (!standaloneNotes) registry.stopAll();
    };
  });

  // The top bar holds every module's entries: when they do not fit the window,
  // the bar tightens its spacing, then shows the entries as icons with the name
  // as a tooltip. The width each step needs is worked out from the entries'
  // text rather than read back from the bar after a change: a webview may
  // restyle later than the read, and the step would then be measured wrong.
  // The numbers mirror the styles of .topbar-inner and .nav-link below.
  const NAV_FIT = [
    { gap: 24, pad: 14, labels: true },
    { gap: 12, pad: 8, labels: true },
    { gap: 12, pad: 10, labels: false },
  ] as const;
  const NAV_ICON = 14;
  const NAV_INNER_GAP = 8;
  let topbarInner = $state<HTMLElement | null>(null);
  let navFit = $state(0);
  let measure: CanvasRenderingContext2D | null = null;
  function textWidth(el: Element | null): number {
    if (!el) return 0;
    const cs = getComputedStyle(el);
    measure ??= document.createElement('canvas').getContext('2d');
    if (!measure) return (el as HTMLElement).offsetWidth;
    measure.font = `${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}`;
    return measure.measureText(el.textContent ?? '').width;
  }
  function fitNav() {
    const el = topbarInner;
    if (!el) return;
    const navEl = el.querySelector<HTMLElement>('.topbar-nav');
    const links = [...el.querySelectorAll<HTMLElement>('.nav-link')];
    const labels = links.map((a) => textWidth(a.querySelector('.nav-label')));
    const badges = links.map((a) => a.querySelector<HTMLElement>('.nav-badge')?.offsetWidth ?? 0);
    const navGap = navEl ? parseFloat(getComputedStyle(navEl).columnGap) || 0 : 0;
    const fixed =
      (el.querySelector<HTMLElement>('.topbar-brand')?.offsetWidth ?? 0) +
      (el.querySelector<HTMLElement>('.topbar-right')?.offsetWidth ?? 0) +
      navGap * Math.max(0, links.length - 1);
    const width = (step: (typeof NAV_FIT)[number]) =>
      fixed +
      2 * step.gap +
      links.reduce(
        (sum, _, i) =>
          sum + 2 * step.pad + NAV_ICON + (step.labels ? NAV_INNER_GAP + labels[i] : 0) + (badges[i] ? NAV_INNER_GAP + badges[i] : 0),
        0,
      );
    let level = 0;
    while (level < NAV_FIT.length - 1 && width(NAV_FIT[level]) > el.clientWidth) level++;
    navFit = level;
  }
  $effect(() => {
    const el = topbarInner;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => fitNav());
    observer.observe(el);
    // The bar is first laid out with the fallback font; the real one is narrower.
    document.fonts?.addEventListener('loadingdone', fitNav);
    void document.fonts?.ready.then(fitNav);
    return () => {
      observer.disconnect();
      document.fonts?.removeEventListener('loadingdone', fitNav);
    };
  });
  // Other entries or other words: measure again once they are on the bar.
  $effect(() => {
    void nav.map((item) => item.id).join();
    void $locale;
    untrack(fitNav);
  });

  function isActive(href: string) {
    if (href === '/') return $page.url.pathname === '/';
    return $page.url.pathname.startsWith(href);
  }

  const fullWidth = $derived(standaloneNotes || registry.fullWidth($page.url.pathname));
</script>

{#snippet toolButton(tool: ToolItem)}
  <button
    class="titlebar-btn"
    onclick={tool.onclick}
    aria-label={$t(tool.title as TranslationKey)}
    title={$t(tool.title as TranslationKey)}
  >
    <Icon name={tool.icon} size={15} />
  </button>
{/snippet}

{#snippet themeIcon()}
  {#if $theme === 'dark'}
    <Icon name="sun" size={15} />
  {:else}
    <Icon name="moon" size={15} />
  {/if}
{/snippet}

<!-- Back from Settings to the product's one module. -->
{#snippet backHome(cls: string)}
  <a href="/" class={cls} title={$t('nav_back_to', { name: homeTitle })}>
    <Icon name="chevron-left" size={15} />
    <span>{homeTitle}</span>
  </a>
{/snippet}

{#snippet titleLead()}
  {#if away && !appLock.gated}
    {@render backHome('titlebar-btn titlebar-back')}
  {/if}
{/snippet}

<!-- The title bar's buttons (client-side decorations): nothing of the app is reachable behind the lock gate (locked, or the recovery flow before its Done). -->
{#snippet titleButtons()}
  {#if !standaloneNotes}
    {#if !appLock.gated}
      {#if single}
        {#each barTools as tool (tool.id)}{@render toolButton(tool)}{/each}
      {/if}
      {#each titleTools as tool (tool.id)}{@render toolButton(tool)}{/each}
    {/if}
    {#if single}
      <button class="titlebar-btn" onclick={toggleTheme} aria-label={$t('theme_toggle')} title={$t('theme_toggle')}>
        {@render themeIcon()}
      </button>
    {/if}
    {#if !appLock.gated}
      <button
        class="titlebar-btn"
        class:active={isActive('/settings')}
        onclick={single ? toggleSettings : () => goto('/settings')}
        aria-label={$t('nav_settings')}
        aria-pressed={single ? isActive('/settings') : undefined}
        title={$t('nav_settings')}
      >
        <Icon name="settings" size={15} />
      </button>
    {/if}
  {/if}
{/snippet}

<WindowFrame
  title={windowTitle ? $t(windowTitle as TranslationKey) : product.name}
  lead={titleLead}
  tools={titleButtons}
>
  <AppLockGate>
  {#if !standaloneNotes && !(single && isCsd)}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <header class="topbar" class:slim={single}>
     <div class="topbar-inner" class:fit-tight={navFit >= 1} bind:this={topbarInner}>
      <a href="/" class="topbar-brand">
        <span class="brand-tile">
          <img src="/logo.png" alt={product.name} class="brand-logo" />
        </span>
        <span class="brand-name">{product.name}</span>
      </a>

      {#if single}
        <!-- One module, one screen: no navigation row; only the way back from Settings. -->
        <nav class="topbar-nav">
          {#if away}{@render backHome('nav-link')}{/if}
        </nav>
      {:else}
      <nav class="topbar-nav">
        {#each nav as item (item.id)}
          {@const badge = item.badge?.() ?? 0}
          <a
            href={item.href}
            class="nav-link"
            class:active={isActive(item.href ?? '/')}
            class:tight={navFit === 1}
            class:icon-only={navFit >= 2}
            title={navFit >= 2 ? $t(item.title as TranslationKey) : undefined}
            aria-label={navFit >= 2 ? $t(item.title as TranslationKey) : undefined}
          >
            <Icon name={item.icon} size={14} />
            <span class="nav-label" class:hidden={navFit >= 2}>{$t(item.title as TranslationKey)}</span>
            {#if badge > 0}<span class="nav-badge">{badge > 99 ? '99+' : badge}</span>{/if}
          </a>
        {/each}
      </nav>
      {/if}

      <div class="topbar-right">
        {#if !isCsd && !single}
          {#each titleTools as tool (tool.id)}
            <button class="theme-toggle" onclick={tool.onclick} title={$t(tool.title as TranslationKey)}>
              <Icon name={tool.icon} size={15} />
            </button>
          {/each}
          <button class="theme-toggle" onclick={() => goto('/settings')} title={$t('nav_settings')}>
            <Icon name="settings" size={15} />
          </button>
        {/if}
        {#if single}
          {#each titleTools as tool (tool.id)}
            <button class="theme-toggle" onclick={tool.onclick} title={$t(tool.title as TranslationKey)}>
              <Icon name={tool.icon} size={15} />
            </button>
          {/each}
        {/if}
        {#each barTools as tool (tool.id)}
          <button class="theme-toggle" onclick={tool.onclick} title={$t(tool.title as TranslationKey)}>
            <Icon name={tool.icon} size={15} />
          </button>
        {/each}
        <button class="theme-toggle" onclick={toggleTheme} title={$t('theme_toggle')}>
          {@render themeIcon()}
        </button>
        {#if single}
          <!-- Named, so it is not taken for a module's own settings gear. -->
          <button class="theme-toggle labelled" class:active={isActive('/settings')} onclick={toggleSettings} aria-pressed={isActive('/settings')}>
            <Icon name="settings" size={15} />
            <span>{$t('nav_settings')}</span>
          </button>
        {/if}
      </div>
     </div>
    </header>
  {/if}
  {#if !standaloneNotes}
    <UpdateBanner />
  {/if}

  <div class="content-wrap">
    <main class="content" class:content--full={fullWidth}>
      {@render children()}
    </main>
    {#if !standaloneNotes}
      <!-- Over the page, inside the frame: the terminal drawers of the ssh module. -->
      {#each contentOverlays as overlay (overlay.key)}
        <overlay.component />
      {/each}
    {/if}
  </div>

  {#if !standaloneNotes}
    <!-- The bottom bars, in flow: the SSH sessions, the dock of running profiles. -->
    {#each bottomOverlays as overlay (overlay.key)}
      <overlay.component />
    {/each}
    <CommandPalette />
  {/if}
  </AppLockGate>
</WindowFrame>

{#if !standaloneNotes && !appLock.gated}
  <!-- Outside the frame: the modules' drawers and banners. -->
  {#each windowOverlays as overlay (overlay.key)}
    <overlay.component />
  {/each}
  <!-- A new data file: which modules to use (section 12). -->
  <ModulesPicker />
{/if}
<!-- The app's confirmation questions (ui/confirm.svelte.ts), above everything else. -->
<ConfirmHost />
<UIInspector />

<style>
  /* Design tokens live in $lib/styles/tokens.css; global reset + primitives
     (.btn, .icon-btn, .form-*, .page, .card, .badge, .tab, .empty-state,
     .spinner, …) live in $lib/styles/base.css. Both imported at top of script. */

  /* The window's frame and title bar are WindowFrame.svelte's. */

  /* The way back from Settings, after the name in the title bar. */
  :global(.titlebar-btn.titlebar-back) {
    margin-left: var(--sp-2);
    padding: 0 10px 0 6px;
    text-decoration: none;
    color: var(--text-2);
  }

  /* ── Top Bar ── */
  .topbar {
    height: var(--topbar-h);
    background: color-mix(in srgb, var(--bg-2) 85%, transparent);
    -webkit-backdrop-filter: blur(10px);
    backdrop-filter: blur(10px);
    border-bottom: 1px solid var(--border);
    display: flex;
    align-items: center;
    padding: 0 var(--rail-pad-x);
    flex-shrink: 0;
    z-index: 10;
  }

  /* One module: the bar holds the name and the tools only. */
  .topbar.slim { height: 48px; }
  .topbar.slim .brand-tile,
  .topbar.slim .brand-logo { width: 26px; height: 26px; }
  .topbar.slim .brand-name { font-size: var(--fs-base); }
  .topbar.slim .nav-link,
  .topbar.slim .theme-toggle { height: 32px; }
  .topbar.slim .theme-toggle { width: 32px; }

  /* Inner rail — aligns topbar contents with the page content rail below */
  .topbar-inner {
    display: flex;
    align-items: center;
    gap: 1.5rem;
    width: 100%;
    max-width: var(--page-max);
    margin-inline: auto;
  }

  .topbar-brand {
    display: flex;
    align-items: center;
    gap: 11px;
    text-decoration: none;
    color: var(--text);
    flex-shrink: 0;
  }

  .brand-tile {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 34px;
    height: 34px;
    flex-shrink: 0;
  }

  .brand-logo {
    width: 34px;
    height: 34px;
    object-fit: contain;
  }

  .brand-name {
    font-weight: var(--fw-bold);
    font-size: 1rem;
    letter-spacing: -0.2px;
  }

  .topbar-nav {
    display: flex;
    align-items: center;
    gap: 0.2rem;
    flex: 1;
  }

  .nav-badge {
    min-width: 18px; height: 18px; padding: 0 5px; border-radius: var(--radius-pill);
    background: var(--accent); color: #fff; font-size: var(--fs-2xs); font-weight: var(--fw-bold);
    display: inline-flex; align-items: center; justify-content: center;
  }
  .nav-link {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 38px;
    padding: 0 14px;
    border-radius: var(--radius);
    text-decoration: none;
    color: var(--text-2);
    font-size: var(--fs-base);
    font-weight: var(--fw-semibold);
    transition: all 0.15s;
  }

  .nav-link:hover { background: var(--surface); color: var(--text); }
  .nav-link.active { background: var(--accent-bg); color: var(--accent-text); }

  /* The steps of fitNav(): tighter spacing, then icons only. Each element
     carries its own class: WebKitGTK does not restyle descendants when only
     an ancestor's class changes under a scoped (`:where`) selector. */
  .topbar-inner.fit-tight { gap: 0.75rem; }
  .nav-link.tight { padding: 0 8px; }
  .nav-link.icon-only { gap: 4px; padding: 0 10px; }
  .nav-label.hidden { display: none; }

  .topbar-right {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-left: auto;
  }

  .theme-toggle {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 38px;
    height: 38px;
    background: var(--surface-3);
    border: 1px solid var(--border);
    border-radius: 9px;
    color: var(--text-soft);
    cursor: pointer;
    transition: all 0.15s;
  }
  .theme-toggle.labelled {
    width: auto;
    gap: 6px;
    padding: 0 10px;
    font-size: var(--fs-sm);
    font-weight: var(--fw-semibold);
  }
  .topbar.slim .theme-toggle.labelled { width: auto; }
  .theme-toggle.active { background: var(--accent-bg); border-color: var(--accent-border); color: var(--accent-text); }
  .theme-toggle:hover:not(:disabled) { background: var(--surface-hover); border-color: var(--border-2); color: var(--text); }

  /* ── Content ── */
  .content {
    flex: 1;
    overflow-y: auto;
    scrollbar-gutter: stable both-edges;  /* symmetric so content rail shares the topbar rail's center line */
    padding: 1.5rem var(--rail-pad-x);
    background: var(--bg);
    min-height: 0;
  }
  .content--full {
    padding: 0.75rem;
    scrollbar-gutter: auto;
  }

  /* The page and, over it, the modules' content overlays (the terminal drawers end where the bottom bars begin). */
  .content-wrap {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
</style>
