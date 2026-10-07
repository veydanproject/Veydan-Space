<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Shown instead of the shell in a degraded start (`app_start_error`). The
  backend answers nothing else in that state, so the screen only reads the
  locale and theme the UI already has. With `error` null the backend did not
  answer the start of the page at all (src/lib/core/start.ts): the screen
  offers to try again. With `hostOs` the UI of the other platform was loaded
  (core/start.ts, `wrongPlatform`): the screen says which, and that the app
  is to be built or installed again. On a computer the root layout puts the
  screen inside the window's frame (`framed`).
-->
<script lang="ts">
  import '@fontsource-variable/manrope/index.css';
  import '@fontsource-variable/jetbrains-mono/index.css';
  import '$lib/core/fonts/cjk.css';
  import '$lib/core/styles/tokens.css';
  import '$lib/core/styles/base.css';
  import { onMount } from 'svelte';
  import type { StartError } from '$lib/core/api';
  import { t, type TranslationKey } from '$lib/core/i18n';
  import { isDesktop, isMobile } from '$lib/core/platform';
  import { isMobileOs } from '$lib/core/start';
  import { syncAndroidChrome, theme } from '$lib/core/theme';
  import Icon from '$lib/core/Icon.svelte';

  interface Props {
    error: StartError | null;
    /** The OS of the device, when this UI was built for the other kind of device. */
    hostOs?: string | null;
    /** Inside the window's frame: the screen fills what is left under the title bar. */
    framed?: boolean;
  }

  let { error, hostOs = null, framed = false }: Props = $props();

  /** A window that can be closed: the device is a computer, whatever the UI was built for. */
  const closable = $derived(hostOs ? !isMobileOs(hostOs) : isDesktop);

  const CAUSE = {
    db_foreign: 'start_error_db_foreign',
    db_dev_schema: 'start_error_db_dev_schema',
  } as const satisfies Record<StartError['code'], TranslationKey>;

  // The sentence keeps its `{path}` slot so the path gets its own element.
  const [before, after] = $derived(error ? $t(CAUSE[error.code] ?? CAUSE.db_foreign).split('{path}') : ['', '']);

  onMount(() => {
    // Theme may have applied before the native chrome bridge was ready.
    syncAndroidChrome($theme);
  });

  /** Window API of the webview, not a backend command. */
  async function closeWindow() {
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    await getCurrentWindow().close();
  }
</script>

<main class="start-error" class:framed>
  <section class="card panel" role="alert">
    <div class="head">
      <span class="mark"><Icon name="alert-triangle" size={22} /></span>
      <h1>{$t('start_error_title')}</h1>
    </div>
    {#if error}
      <p class="cause">{before}<code class="path">{error.path}</code>{after}</p>
      <p class="note">{$t('start_error_untouched')}</p>
      <p class="note">{$t('start_error_fix')}</p>
    {:else if hostOs}
      <p class="cause">{isMobile ? $t('start_error_wrong_ui_phone') : $t('start_error_wrong_ui_desktop')}</p>
      <p class="note">{$t('start_error_wrong_ui_fix')}</p>
    {:else}
      <p class="cause">{$t('start_error_no_answer')}</p>
    {/if}
    {#if closable || !error}
      <div class="actions">
        {#if closable}
          <button class="btn btn-ghost" onclick={closeWindow}>{$t('start_error_close')}</button>
        {/if}
        {#if !error && !hostOs}
          <button class="btn btn-primary" onclick={() => location.reload()}>{$t('start_error_retry')}</button>
        {/if}
      </div>
    {/if}
  </section>
</main>

<style>
  .start-error {
    position: fixed;
    inset: 0;
    display: flex;
    overflow-y: auto;
    padding: var(--sp-6) var(--sp-4);
    padding-top: max(var(--sp-6), var(--sat, env(safe-area-inset-top, 0px)));
    padding-bottom: max(var(--sp-6), var(--sab, env(safe-area-inset-bottom, 0px)));
    background: var(--bg);
    color: var(--text);
  }
  /* Under the title bar of the window's frame. */
  .start-error.framed {
    position: static;
    flex: 1;
    min-height: 0;
  }

  /* Auto margins centre the card and still let a tall one scroll from its top. */
  .panel {
    width: min(560px, 100%);
    margin: auto;
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }

  .head {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
  }

  .mark {
    flex-shrink: 0;
    width: 44px;
    height: 44px;
    border-radius: var(--radius-md);
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--danger-bg);
    color: var(--danger-text);
  }

  h1 {
    font-size: var(--fs-lg);
    font-weight: var(--fw-bold);
    line-height: var(--lh-tight);
  }

  .cause {
    color: var(--text);
  }

  .note {
    color: var(--text-body);
  }

  .path {
    padding: 1px 6px;
    border-radius: var(--radius-xs);
    background: var(--surface-2);
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
    color: var(--text-body);
    overflow-wrap: anywhere;
    -webkit-box-decoration-break: clone;
    box-decoration-break: clone;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--sp-2);
    margin-top: var(--sp-2);
  }
</style>
