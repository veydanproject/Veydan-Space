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
  import { hub } from '$lib/core/mobile/hub.svelte';
  import HubSheet from '$lib/core/mobile/HubSheet.svelte';
  import AppLockGate from '$lib/core/lock/AppLockGate.svelte';
  import LinkInbox from '$lib/core/links/LinkInbox.svelte';
  import { appLock } from '$lib/core/lock/store.svelte';
  import { leavesOff, registry } from '$lib/core/registry';
  import Wizard from '$lib/core/onboarding/Wizard.svelte';
  import ConfirmHost from '$lib/core/ui/ConfirmHost.svelte';
  import ToastHost from '$lib/core/ui/ToastHost.svelte';
  import { startUpdateCheck } from '$lib/core/store/update-check.svelte';
  import { distribution, loadDistribution } from '$lib/core/store/distribution.svelte';
  import { t } from '$lib/core/mobile/i18n';

  let { children }: { children: Snippet } = $props();

  // A screen that failed to show: the way out is the start screen of the
  // product, in its place, so back does not lead to it again.
  const home = (registry.only && registry.homeRoute(registry.only)) ?? '/';
  async function goHome(reset: () => void) {
    await goto(home, { replaceState: true }).catch(() => {});
    reset();
  }

  // The hub chooses between the product's apps: a product of one module has
  // none to choose between, and its bar is the module's screens (11.7).
  hub.available = registry.only === undefined;
  const withHub = hub.available;
  // A screen whose header carries the apps button has no centre button in its bar.
  const hubInBar = $derived(withHub && !registry.hubInHeader(page.url));

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
    // An app installed by a store is updated by the store and never checks.
    let stopUpdateCheck = () => {};
    let gone = false;
    void loadDistribution().then(() => {
      if (!gone && !distribution.fromStore) stopUpdateCheck = startUpdateCheck();
    });
    return () => {
      gone = true;
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
      <BottomNav {items} activeId={active} hub={hubInBar} hubActive={hub.isOpen} onhub={() => hub.open()} />
    {/if}
  </AppLockGate>
</div>

{#if !appLock.gated}
  {#each overlays as overlay (overlay.key)}
    <overlay.component />
  {/each}
  <!-- A link clicked outside the app, read once it is unlocked. -->
  <LinkInbox />
  {#if withHub}
    <HubSheet open={hub.isOpen} onclose={() => hub.close()} />
  {/if}
  <!-- A new data file: the first start (section 12). -->
  <Wizard />
{/if}
<!-- The app's confirmation questions (ui/confirm.svelte.ts), above everything else. -->
{#if !appLock.gated}
  <ConfirmHost />
{/if}
<ToastHost />

<style>
  .shell {
    height: 100dvh;
    display: flex;
    flex-direction: column;
    /* Nothing of the shell may make the document scroll: a focus or a
       scroll-into-view would move the header under the status area. */
    overflow: clip;
    background: var(--bg-image), var(--bg);
  }
  /* Home has no header: the strip under the status bar is the page's own colour. */
  .shell.home .screen { background: transparent; }
  .screen {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    padding-top: var(--sat);
    background: linear-gradient(var(--m-head), var(--m-head)) top / 100% var(--sat) no-repeat;
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
