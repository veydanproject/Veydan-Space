<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import '@fontsource-variable/manrope/index.css';
  import '@fontsource-variable/jetbrains-mono/index.css';
  import '$lib/core/fonts/cjk.css';
  import '$lib/core/styles/tokens.css';
  import '$lib/core/styles/base.css';
  import '$lib/core/styles/mobile.css';
  import { onMount, setContext, untrack, type Snippet } from 'svelte';
  import { page } from '$app/state';
  import { goto } from '$app/navigation';
  import { syncAndroidChrome, theme } from '$lib/core/theme';
  import { navActive, navItems } from '$lib/core/mobile/nav';
  import BottomNav from '$lib/core/mobile/BottomNav.svelte';
  import { hub } from '$lib/core/mobile/hub.svelte';
  import { atHome, split, takeSystemBack } from '$lib/core/ui/split.svelte';
  import { platform } from '$lib/core/platform';
  import HubSheet from '$lib/core/mobile/HubSheet.svelte';
  import { startIosKeyboard } from '$lib/core/mobile/ios-keyboard';
  import { installScrolling } from '$lib/core/scrolling';
  import AppLockGate from '$lib/core/lock/AppLockGate.svelte';
  import Icon from '$lib/core/Icon.svelte';
  import LinkInbox from '$lib/core/links/LinkInbox.svelte';
  import { appLock } from '$lib/core/lock/store.svelte';
  import { leavesOff, registry } from '$lib/core/registry';
  import Wizard from '$lib/core/onboarding/Wizard.svelte';
  import ConfirmHost from '$lib/core/ui/ConfirmHost.svelte';
  import ToastHost from '$lib/core/ui/ToastHost.svelte';
  import { startUpdateCheck } from '$lib/core/store/update-check.svelte';
  import { distribution, loadDistribution } from '$lib/core/store/distribution.svelte';
  import { t } from '$lib/core/mobile/i18n';
  import { derived } from 'svelte/store';

  let { children }: { children: Snippet } = $props();

  // A screen that failed to show: the way out is the start screen of the
  // product, in its place, so back does not lead to it again.
  const home = (registry.only && registry.homeRoute(registry.only)) ?? '/';
  async function goHome(reset: () => void) {
    await goto(home, { replaceState: true }).catch(() => {});
    reset();
  }

  // The hub chooses between the product's apps: a product of one module has
  // none to choose between, and its bar is the module's screens (11.7). It is
  // opened by the logo in the header of a screen (core/ui/HubButton); no bar has one.
  hub.available = registry.only === undefined;
  const withHub = hub.available;
  // A product of one module has no Settings in its tab bar: the avatar of the module's tabs opens the app's settings (messenger/mobile/MeButton).
  if (registry.only) setContext('app-settings', { href: '/settings', label: derived(t, (tr) => tr('app_settings')) });

  // The bar for this screen: a module's own or the root one (platform-spec 11.7).
  const items = $derived(navItems(page.url));
  const active = $derived(items ? navActive(page.url.pathname, items) : '');
  // A wide window (core/ui/split): the bar is a column of sections at the
  // left, a module's list stands beside the screen. The bar of a screen beside
  // a list is the list's: an open item's own bar may be none (a chat).
  const pair = $derived(split.of(page.url));
  const rail = $derived(split.on ? (pair ? navItems(new URL(pair.home, page.url)) : items) : null);
  const railActive = $derived(rail ? navActive(pair ? new URL(pair.home, page.url).pathname : page.url.pathname, rail) : '');
  // On the list's own address the list is at the left already: the right part is empty.
  const blank = $derived(!!pair && atHome(page.url, pair));
  // An item beside its list: the system «Назад» empties the right part, as the hidden arrow did (split.takeSystemBack).
  const besidePair = $derived(platform === 'android' && pair && !blank ? pair : undefined);
  $effect(() => {
    const def = besidePair;
    if (!def) return;
    return untrack(() => takeSystemBack(def, () => page.state));
  });
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
    const stopIosKeyboard = startIosKeyboard();
    const uninstallScrolling = installScrolling();
    // A phone has no updater: the product's latest.json, once a day (14.3).
    // An app installed by a store is updated by the store and never checks.
    let stopUpdateCheck = () => {};
    let gone = false;
    void loadDistribution().then(() => {
      if (!gone && !distribution.fromStore) stopUpdateCheck = startUpdateCheck();
    });
    return () => {
      gone = true;
      stopIosKeyboard();
      uninstallScrolling();
      stopUpdateCheck();
      registry.stopAll();
    };
  });
</script>

<div class="shell" class:home={page.url.pathname === '/'} class:no-bar={!items} class:wide={split.on}>
  <AppLockGate>
    <!-- The screen keeps its place in the template whatever stands beside it: folding does not mount it again. -->
    {#if rail}
      <BottomNav rail items={rail} activeId={railActive} />
    {/if}
    {#if pair}
      <div class="split-list">
        <!-- Lists of one component (the chats, the groups, a section) are lists of their own, as on a phone. -->
        {#key pair.home}
          <pair.list {...pair.props} />
        {/key}
      </div>
    {/if}
    <div class="screen">
      <svelte:boundary onerror={(e) => console.error('screen failed', e)}>
        {#if !blank}
          {@render children()}
        {:else if pair?.empty}
          <div class="empty-state split-empty">
            <Icon name={pair.empty.icon} size={28} />
            {#if pair.empty.title}<p>{$t(pair.empty.title)}</p>{/if}
            {#if pair.empty.text}<span>{$t(pair.empty.text)}</span>{/if}
          </div>
        {/if}
        {#snippet failed(_error, reset)}
          <div class="m-page">
            <div class="m-empty">
              <span class="empty-icon"><Icon name="alert-triangle" size={24} /></span>
              <p>{$t('screen_failed')}</p>
              <div class="actions">
                <button type="button" class="btn btn-ghost btn-sm" onclick={reset}><Icon name="arrow-forward-up" size={16} />{$t('common_retry')}</button>
                <button type="button" class="btn btn-ghost btn-sm" onclick={() => goHome(reset)}>{$t('nav_home')}</button>
              </div>
            </div>
          </div>
        {/snippet}
      </svelte:boundary>
    </div>
    {#if items && !split.on}
      <BottomNav {items} activeId={active} />
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
  .screen {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    padding-top: var(--sat);
  }
  /* Wide: the column of sections, the list, the screen in a row. Nothing on
     the columns makes a containing block (no transform, no contain): the
     menus, the suggestions and the glass measure against the window. */
  .shell.wide { flex-direction: row; }
  /* The screen holds what stands at its own foot (the note's toolbar), as the list does its «+». */
  .shell.wide .screen { min-width: 0; position: relative; }
  .split-list {
    position: relative;
    flex: 0 0 clamp(360px, 41vw, 400px);
    min-width: 0;
    display: flex;
    flex-direction: column;
    padding-top: var(--sat);
    border-right: 1px solid var(--border);
  }
  .split-list > :global(.m-page) {
    flex: 1;
    min-height: 0;
  }
  .screen > :global(.m-page) {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  /* The empty right part says what to do, as on the desktop: an icon, a title and a line, grey, in the middle. */
  .split-empty {
    height: 100%;
    box-sizing: border-box;
    padding: var(--sp-6);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--sp-2);
    text-align: center;
    color: var(--text-2);
  }
  .split-empty p { margin: 0; font-weight: var(--fw-semibold); color: var(--text-2); }
  .split-empty span { font-size: var(--fs-sm); color: var(--text-3); max-width: 360px; }
  /* Retry first, the way home beside it. */
  .actions { display: flex; flex-wrap: wrap; justify-content: center; gap: var(--sp-2); }
  .screen > :global(.editor) {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
</style>
