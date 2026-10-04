<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import Icon from '$lib/core/Icon.svelte';
  import { t } from '$lib/core/mobile/i18n';
  import { registry } from '$lib/core/registry';
  import type { NavItem } from '$lib/core/module';
  import { product } from '$lib/core/product';
  import BottomSheet from './BottomSheet.svelte';

  let { open, onclose }: { open: boolean; onclose: () => void } = $props();

  // Home, the modules' entries, then Settings; the current one is the entry
  // whose route the path starts with (Home only on `/`). A module with a bar
  // of its own (the notes) has no Home tab: this is its way back to Home.
  const apps = $derived<NavItem[]>([
    { id: 'home', title: 'nav_home', icon: 'home', href: '/' },
    ...registry.nav(),
    { id: 'settings', title: 'app_settings', icon: 'settings', href: '/settings' },
  ]);
  let current = $derived(
    apps.find((a) => a.href && (a.href === '/' ? page.url.pathname === '/' : page.url.pathname === a.href || page.url.pathname.startsWith(a.href + '/')))?.id,
  );

  // Home is asked for by name: the default app, which `/` would open, is not
  // meant. The apps are roots of the bar: one replaces the other in the history.
  function pick(app: NavItem) {
    onclose();
    void goto(app.id === 'home' ? '/?home' : (app.href ?? '/'), { replaceState: true });
  }
</script>

<BottomSheet {open} {onclose}>
  <div class="head">
    <img src="/logo.png" alt="" />
    <h2>{product.name}</h2>
    <p>{$t('hub_sub')}</p>
  </div>
  <div class="m-grid">
    {#each apps as app (app.id)}
      <button type="button" class="m-tile" class:active={current === app.id} onclick={() => pick(app)}>
        <Icon name={app.icon} size={24} />
        <span class="label">{$t(app.title)}</span>
      </button>
    {/each}
  </div>
</BottomSheet>

<style>
  .head {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding-bottom: var(--sp-2);
  }
  .head img { width: 40px; height: 40px; object-fit: contain; margin-bottom: var(--sp-1); }
  h2 { margin: 0; font-size: 20px; font-weight: 800; letter-spacing: -0.02em; }
  p { margin: 0; color: var(--text-2); font-size: 13px; }
  .m-grid { padding-bottom: var(--sp-2); }
</style>
