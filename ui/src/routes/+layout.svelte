<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import type { Snippet } from 'svelte';
  import { isMobile } from '$lib/core/platform';
  import { api, type StartError } from '$lib/core/api';
  import { registry } from '$lib/core/registry';
  import { product } from '$lib/core/product';
  import { answerWithin, isMobileOs, retryOnce, START_TIMEOUT_MS, started as startWorked, wrongPlatform } from '$lib/core/start';

  let { children }: { children: Snippet } = $props();

  const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

  // The window, the task switcher and the screen reader name the product.
  if (typeof document !== 'undefined') document.title = product.name;

  /**
   * The screen shown instead of the shell. On a computer it stands inside the
   * window's frame, so the window can still be moved and closed. `hostOs` is
   * known when the UI of the other platform was loaded: the frame follows the
   * device, not the UI.
   */
  async function errorView(error: StartError | null, hostOs?: string) {
    const desktop = hostOs ? !isMobileOs(hostOs) : !isMobile;
    const [{ default: Screen }, Frame] = await Promise.all([
      import('$lib/core/StartErrorScreen.svelte'),
      desktop ? import('$lib/core/desktop/WindowFrame.svelte').then((m) => m.default) : null,
    ]);
    return { error, Screen, Frame, hostOs: hostOs ?? null };
  }

  // The backend is asked once, before anything is mounted: in a degraded start
  // no other command works, so only the error screen is fetched. Otherwise the
  // shell of the build's platform is fetched — `virtual:veydan-modules/shell`:
  // the other platform's shell is not in the build at all, so neither its CSS
  // nor its preload can reach this UI (ui/vite-veydan-modules.js) —, and the
  // registry of the product's modules is loaded before the shell and the
  // pages read it.
  async function start() {
    const error = await api.app.startError().catch((e) => {
      console.error('app_start_error failed', e);
      return null;
    });
    if (error) return errorView(error);
    // The phone UI in a desktop window (or the reverse) is a build that took
    // the other platform's pages: say so instead of mounting the wrong shell.
    if (isTauri) {
      const host = await api.system.hostInfo().catch(() => null);
      if (wrongPlatform(isMobile, host?.os)) {
        console.error(`the UI was built for ${isMobile ? 'a phone' : 'a computer'}, the app runs on ${host?.os}`);
        return errorView(null, host?.os);
      }
    }
    const [{ default: Shell }] = await Promise.all([
      import('virtual:veydan-modules/shell'),
      registry.load(),
    ]);
    return { Shell };
  }

  // A start that is not answered in time, or fails, reloads the page once;
  // a second failure shows a screen with a retry instead of a blank page.
  async function startOrSayWhy() {
    try {
      const view = await answerWithin(start(), START_TIMEOUT_MS);
      startWorked();
      return view;
    } catch (e) {
      console.error('the start of the page failed', e);
      if (retryOnce()) return new Promise<never>(() => {});
      return errorView(null);
    }
  }

  const started = startOrSayWhy();
</script>

{#await started then view}
  {#if 'Shell' in view}
    <view.Shell>{@render children()}</view.Shell>
  {:else if view.Frame}
    <view.Frame title={product.name}>
      <view.Screen error={view.error} hostOs={view.hostOs} framed />
    </view.Frame>
  {:else}
    <view.Screen error={view.error} hostOs={view.hostOs} />
  {/if}
{/await}
