<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import '@fontsource-variable/manrope/index.css';
  import '@fontsource-variable/jetbrains-mono/index.css';
  import '$lib/core/fonts/cjk.css';
  import '$lib/core/styles/tokens.css';
  import '$lib/core/styles/base.css';
  import '$lib/core/styles/mobile.css';
  import { onMount, type Snippet } from 'svelte';
  import { page } from '$app/state';
  import { goto } from '$app/navigation';
  import { syncAndroidChrome, theme } from '$lib/core/theme';
  import { navActive, navItems } from '$lib/core/mobile/nav';
  import BottomNav from '$lib/core/mobile/BottomNav.svelte';
  import HubSheet from '$lib/core/mobile/HubSheet.svelte';
  import AppLockGate from '$lib/core/lock/AppLockGate.svelte';
  import { appLock } from '$lib/core/lock/store.svelte';
  import { leavesOff, registry } from '$lib/core/registry';
  import ModulesPicker from '$lib/core/ModulesPicker.svelte';
  import ConfirmHost from '$lib/core/ui/ConfirmHost.svelte';
  import { startUpdateCheck } from '$lib/core/store/update-check.svelte';
  import { t } from '$lib/core/mobile/i18n';

  let { children }: { children: Snippet } = $props();

  // A screen that failed to show: the way out is the start screen of the
  // product, in its place, so back does not lead to it again.
  const home = (registry.only && registry.homeRoute(registry.only)) ?? '/';
  async function goHome(reset: () => void) {
    await goto(home, { replaceState: true }).catch(() => {});
    reset();
  }

  let hubOpen = $state(false);
  // The hub chooses between the product's apps: a product of one module has
  // none to choose between, and its bar is the module's screens (11.7).
  const withHub = registry.only === undefined;

  // The bar for this screen: a module's own or the root one (platform-spec 11.7).
  const items = $derived(navItems(page.url));
  const active = $derived(items ? navActive(page.url.pathname, items) : '');
  // The modules' sheets and banners, outside the screen; a module the user
  // switches off takes its own away (section 12).
  const overlays = $derived(registry.overlays('window'));

  // The screen of a module the user switched off: the shell goes to `/`.
  $effect(() => {
    if (leavesOff(page.url.pathname)) void goto('/', { replaceState: true });
  });

  // Mobile defaults to the light theme; the shared store defaults to dark and
  // has already persisted it, so a one-time flag marks the first launch.
  if (typeof localStorage !== 'undefined' && !localStorage.getItem('m_theme_init')) {
    localStorage.setItem('m_theme_init', '1');
    theme.set('light');
  }

  // Subscribing applies data-theme to <body>.
  $effect(() => {
    void $theme;
  });

  // Android freezes the process in background; catch up when the app comes back.
  onMount(() => {
    // Theme may have applied before the native chrome bridge was ready.
    syncAndroidChrome($theme);
    void appLock.listen();
    void registry.startAll();
    // A phone has no updater: the product's latest.json, once a day (14.3).
    const stopUpdateCheck = startUpdateCheck();
    return () => {
      stopUpdateCheck();
      registry.stopAll();
    };
  });
</script>

<div class="shell" class:home={page.url.pathname === '/'}>
  <AppLockGate>
    <div class="screen">
      <svelte:boundary onerror={(e) => console.error('screen failed', e)}>
        {@render children()}
        {#snippet failed(_error, reset)}
          <div class="m-page">
            <div class="m-empty">
              <p>{$t('screen_failed')}</p>
              <button type="button" class="m-btn-grad" onclick={() => goHome(reset)}>{$t('nav_home')}</button>
            </div>
          </div>
        {/snippet}
      </svelte:boundary>
    </div>
    {#if items}
      <BottomNav {items} activeId={active} hub={withHub} hubActive={hubOpen} onhub={() => (hubOpen = true)} />
    {/if}
  </AppLockGate>
</div>

{#if !appLock.gated}
  {#each overlays as overlay (overlay.key)}
    <overlay.component />
  {/each}
  {#if withHub}
    <HubSheet open={hubOpen} onclose={() => (hubOpen = false)} />
  {/if}
  <!-- A new data file: which modules to use (section 12). -->
  <ModulesPicker />
{/if}
<!-- The app's confirmation questions (ui/confirm.svelte.ts), above everything else. -->
<ConfirmHost />

<style>
  .shell {
    height: 100dvh;
    display: flex;
    flex-direction: column;
    /* Nothing of the shell may make the document scroll: a focus or a
       scroll-into-view would move the header under the status area. */
    overflow: clip;
    background: var(--bg);
  }
  /* Home has no header: the strip under the status bar is the page's own colour. */
  .shell.home .screen { background: var(--bg); }
  .screen {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    padding-top: var(--sat);
    background: var(--m-nav);
  }
  .screen > :global(.m-page) {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  .screen > :global(.editor) {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
</style>
