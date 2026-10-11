<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The camera of a phone over the whole screen, to read a QR code. The
  plugin draws the camera behind the webview, so while it scans everything
  of the app goes (`html.scan-cover`, mobile.css) and this layer, which lives
  outside the shell, keeps only what a person needs: where to aim, what is
  being scanned and the ways out. Without the camera (no permission, no
  plugin) the layer says so and offers the way by hand.
-->
<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { portal } from '$lib/core/portal';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';

  let {
    title,
    hint,
    manual,
    onresult,
    onclose,
  }: {
    title: string;
    hint: string;
    /** The words of the way without the camera («Ввести 8 слов»). */
    manual: string;
    onresult: (content: string) => void;
    onclose: () => void;
  } = $props();

  type Phase = 'starting' | 'scanning' | 'denied' | 'failed';
  let phase = $state<Phase>('starting');
  /** The plugin is scanning: it has to be told to stop. */
  let active = false;
  /** The layer is closing: what the plugin says after that is not news. */
  let closing = false;

  function cover(on: boolean) {
    document.documentElement.classList.toggle('scanning', on);
    document.documentElement.classList.toggle('scan-cover', on);
  }

  async function start() {
    phase = 'starting';
    try {
      const scanner = await import('@tauri-apps/plugin-barcode-scanner');
      let perm = await scanner.checkPermissions();
      // Anything but a final answer may still be asked: after one refusal Android says `prompt-with-rationale`.
      if (perm !== 'granted' && perm !== 'denied') {
        perm = await scanner.requestPermissions();
        // Let the permission activity finish before the camera binds to ours.
        await new Promise((r) => setTimeout(r, 400));
      }
      if (closing) return;
      if (perm !== 'granted') {
        phase = 'denied';
        return;
      }
      cover(true);
      active = true;
      phase = 'scanning';
      const result = await scanner.scan({ windowed: true, formats: [scanner.Format.QRCode] });
      active = false;
      cover(false);
      if (!closing) onresult(result.content);
    } catch (e) {
      active = false;
      cover(false);
      if (closing) return;
      console.error('QR scan failed:', e);
      phase = 'failed';
    }
  }

  async function stop() {
    closing = true;
    cover(false);
    if (!active) return;
    active = false;
    try {
      const scanner = await import('@tauri-apps/plugin-barcode-scanner');
      await scanner.cancel();
    } catch {
      // Nothing was scanning.
    }
  }

  async function close() {
    await stop();
    onclose();
  }

  async function openSettings() {
    try {
      const scanner = await import('@tauri-apps/plugin-barcode-scanner');
      await scanner.openAppSettings();
    } catch {
      // No settings to open here.
    }
  }

  /** Back from the settings of the system: the permission may be there now. */
  function onVisible() {
    if (document.visibilityState === 'visible' && phase === 'denied' && !closing) void start();
  }

  onMount(() => {
    document.addEventListener('visibilitychange', onVisible);
    void start();
  });
  onDestroy(() => {
    document.removeEventListener('visibilitychange', onVisible);
    void stop();
  });
</script>

<div class="scanner" class:live={phase === 'scanning'} use:portal role="dialog" aria-modal="true" aria-label={title}>
  <div class="top">
    <button class="round" type="button" onclick={close} aria-label={$t('common_close')}><Icon name="x" size={22} /></button>
    <h1>{title}</h1>
  </div>

  {#if phase === 'denied' || phase === 'failed'}
    <div class="stop">
      <span class="mark"><Icon name="camera" size={28} /></span>
      <p class="lead">{phase === 'denied' ? $t('cloud_scan_denied') : $t('cloud_scan_failed')}</p>
      <div class="buttons">
        {#if phase === 'denied'}
          <button class="btn btn-primary" type="button" onclick={openSettings}>{$t('cloud_scan_open_settings')}</button>
        {:else}
          <button class="btn btn-primary" type="button" onclick={start}>{$t('cloud_scan_retry')}</button>
        {/if}
        <button class="btn btn-ghost" type="button" onclick={close}>{manual}</button>
      </div>
    </div>
  {:else}
    <div class="aim">
      <div class="frame" class:idle={phase === 'starting'}>
        <span class="corner tl"></span><span class="corner tr"></span><span class="corner bl"></span><span class="corner br"></span>
        {#if phase === 'scanning'}<span class="line"></span>{/if}
      </div>
      <p class="hint">{phase === 'starting' ? $t('cloud_scan_starting') : hint}</p>
    </div>
    <div class="bottom">
      <button class="pill" type="button" onclick={close}>{manual}</button>
    </div>
  {/if}
</div>

<style>
  .scanner {
    position: fixed; inset: var(--overlay-inset, 0); z-index: var(--z-max);
    display: flex; flex-direction: column;
    padding: calc(var(--sat, 0px) + var(--sp-2)) var(--sp-4) calc(var(--sab, 0px) + var(--sp-6));
    background: var(--bg); color: var(--text);
  }
  /* Over the camera: no background of its own, the words in white. */
  .scanner.live { background: transparent; color: #fff; }

  .top { position: relative; z-index: 1; display: flex; align-items: center; gap: var(--sp-3); min-height: 48px; }
  .top h1 { margin: 0; font-size: 18px; font-weight: var(--fw-bold); }
  .round {
    display: inline-flex; align-items: center; justify-content: center; width: 44px; height: 44px; border-radius: 50%;
    border: 0; background: var(--surface-3); color: inherit;
  }
  .live .round { background: var(--backdrop); }

  .aim { flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: var(--sp-6); }
  .frame {
    position: relative; width: min(72vw, 300px); aspect-ratio: 1; border-radius: 28px;
    /* Everything outside the frame is dimmed: the frame is the one clear place. */
    box-shadow: 0 0 0 100vmax var(--backdrop);
  }
  .frame.idle { box-shadow: none; background: var(--surface-3); }
  .corner { position: absolute; width: 44px; height: 44px; border: 4px solid #fff; }
  .idle .corner { border-color: var(--border-strong); }
  .tl { top: 0; left: 0; border-right: 0; border-bottom: 0; border-top-left-radius: 28px; }
  .tr { top: 0; right: 0; border-left: 0; border-bottom: 0; border-top-right-radius: 28px; }
  .bl { bottom: 0; left: 0; border-right: 0; border-top: 0; border-bottom-left-radius: 28px; }
  .br { bottom: 0; right: 0; border-left: 0; border-top: 0; border-bottom-right-radius: 28px; }
  .line {
    position: absolute; left: 12%; right: 12%; top: 10%; height: 2px; border-radius: 2px;
    background: var(--accent); box-shadow: 0 0 12px var(--accent);
    animation: sweep 2.4s ease-in-out infinite alternate;
  }
  @keyframes sweep { to { top: 90%; } }
  @media (prefers-reduced-motion: reduce) { .line { animation: none; top: 50%; } }

  .hint { position: relative; z-index: 1; margin: 0; max-width: 300px; text-align: center; font-size: 15px; line-height: 1.45; font-weight: var(--fw-medium); }
  /* Over a bright picture the words keep a ground of their own. */
  .live .hint { padding: var(--sp-2) var(--sp-4); border-radius: var(--radius-md); background: var(--backdrop); }
  .bottom { position: relative; z-index: 1; display: flex; justify-content: center; }
  .pill {
    min-height: 48px; padding: 0 var(--sp-6); border-radius: var(--radius-pill); border: 0;
    background: var(--surface-3); color: inherit; font: inherit; font-size: 15px; font-weight: var(--fw-semibold);
  }
  .live .pill { background: var(--backdrop); }

  .stop { flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: var(--sp-4); text-align: center; }
  .mark {
    display: inline-flex; align-items: center; justify-content: center; width: 64px; height: 64px; border-radius: 50%;
    background: var(--warn-bg); color: var(--warn-text); border: 1px solid var(--warn-border);
  }
  .lead { margin: 0; max-width: 300px; font-size: 15px; line-height: 1.45; color: var(--text-2); }
  .buttons { display: flex; flex-direction: column; gap: var(--sp-2); align-self: stretch; }
  .buttons .btn { width: 100%; justify-content: center; min-height: 48px; font-size: 15px; }
</style>
