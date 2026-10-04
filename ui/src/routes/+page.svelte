<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  `/` opens the home screen of the default module (platform-spec 11.7). On
  the phone that is Home — the grid of modules, or the module the user chose;
  a product of one module opens that module. On the desktop it is the default
  module's own home screen when it has one (the workspaces of the browser
  module), else the module's first route.
-->
<script lang="ts">
  import { goto } from '$app/navigation';
  import { isMobile } from '$lib/core/platform';
  import { registry } from '$lib/core/registry';
  import MobileHome from '$lib/core/mobile/HomePage.svelte';

  // A module switched off is not the default one: `/` follows the switches.
  const home = $derived(isMobile ? undefined : registry.defaultModule());

  $effect(() => {
    if (isMobile || home?.home) return;
    const route = home && registry.homeRoute(home);
    if (route) void goto(route, { replaceState: true });
  });
</script>

{#if isMobile}
  <MobileHome />
{:else if home?.home}
  <home.home />
{/if}
