<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { goto } from '$app/navigation';
  import { onMount } from 'svelte';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/mobile/i18n';
  import { page } from '$app/state';
  import { loadDefaultApp, registry } from '$lib/core/registry';
  import { product } from '$lib/core/product';
  import SetupCard from '$lib/core/onboarding/SetupCard.svelte';
  import { updateOffered } from '$lib/core/store/update-check.svelte';

  type Greeting = 'home_morning' | 'home_day' | 'home_evening' | 'home_night';

  let ready = $state(false);
  const search = registry.search();

  function greetingNow(): Greeting {
    const h = new Date().getHours();
    if (h < 5) return 'home_night';
    if (h < 12) return 'home_morning';
    if (h < 18) return 'home_day';
    return 'home_evening';
  }
  // The phone keeps the screen for hours: the greeting is read again when the app comes back.
  let greeting = $state<Greeting>(greetingNow());
  $effect(() => {
    const refresh = () => {
      if (document.visibilityState === 'visible') greeting = greetingNow();
    };
    document.addEventListener('visibilitychange', refresh);
    window.addEventListener('focus', refresh);
    return () => {
      document.removeEventListener('visibilitychange', refresh);
      window.removeEventListener('focus', refresh);
    };
  });

  onMount(async () => {
    // A module may take the user somewhere first (the chat of a tapped
    // notification that started the app); then the default app, else Home.
    for (const m of registry.active) {
      if (await m.resume?.()) return;
    }
    // The app the user chose; a product of one module has no Home, and `/` is
    // that module (platform-spec 11.7).
    const only = registry.only;
    // `/?home` asks for Home itself (the hub's Home tile), past the default app.
    const wanted = page.url.searchParams.has('home') ? undefined : registry.navItem(loadDefaultApp())?.href;
    const route = wanted ?? (only && registry.homeRoute(only));
    if (route) {
      goto(route, { replaceState: true });
      return;
    }
    ready = true;
  });
</script>

{#if ready}
  <div class="m-page home">
    <header class="brand">
      <img src="/logo.png" alt="" class="logo" />
      <h1>{product.name}</h1>
      <p>{$t(greeting)}</p>
    </header>

    {#if search}
      <div class="search-field fill">
        <Icon name="search" size={16} />
        <a class="search-link" href={search}>{$t('home_search')}</a>
      </div>
    {/if}

    <!-- What the first start left to set up. -->
    <SetupCard />
    <div class="m-grid">
      <!-- No tile is highlighted: nothing is selected on Home. -->
      {#each registry.nav() as app (app.id)}
        <a href={app.href} class="m-tile">
          <Icon name={app.icon} size={24} />
          <span class="label">{$t(app.title)}</span>
        </a>
      {/each}
      <!-- Home has no bar: Settings is the last tile. -->
      <a href="/settings" class="m-tile">
        {#if updateOffered()}<span class="status-dot tile-dot" aria-hidden="true"></span>{/if}
        <Icon name="settings" size={24} />
        <span class="label">{$t('app_settings')}</span>
      </a>
    </div>
  </div>
{/if}

<style>
  .home { padding-top: var(--sp-8); }
  .brand {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-0-5);
    margin-bottom: var(--sp-6);
  }
  .logo { width: 64px; height: 64px; object-fit: contain; margin-bottom: var(--sp-2); }
  h1 { margin: 0; font-size: var(--fs-xl); font-weight: var(--fw-bold); letter-spacing: var(--ls-tight); line-height: var(--lh-tight); }
  p { margin: 0; color: var(--text-2); font-size: var(--fs-body); }
  .search-field { margin-bottom: var(--sp-5); }
  /* A new version (design 18): the dot of design 06 at the tile's corner, as on the tab of Settings in a bar. */
  .m-tile { position: relative; }
  .tile-dot { position: absolute; top: var(--sp-2); right: var(--sp-2); background: var(--accent); }
</style>
