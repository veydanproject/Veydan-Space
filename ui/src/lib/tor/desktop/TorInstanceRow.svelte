<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- One Tor instance: state, progress, address, and its actions and log. -->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import { t, type TranslationKey } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { formatError } from '$lib/core/utils';
  import { api } from '$lib/tor/api';
  import { clampPercent } from '$lib/tor/format';
  import { exitLabel, socksAddress, torErrorText } from '$lib/tor/settings';
  import type { InstanceInfo, InstanceState } from '$lib/tor/types';

  let { info }: { info: InstanceInfo } = $props();

  const STATE_LABEL: Record<InstanceState, TranslationKey> = {
    starting: 'tor_state_starting',
    ready: 'tor_state_ready',
    restarting: 'tor_state_restarting',
    stopping: 'tor_state_stopping',
    failed: 'tor_state_failed',
  };
  const STATE_BADGE: Record<InstanceState, string> = {
    starting: 'badge-accent',
    ready: 'badge-ok',
    restarting: 'badge-accent',
    stopping: 'badge-warn',
    failed: 'badge-danger',
  };

  const LOG_POLL_MS = 2000;

  let busy = $state(false);
  let message = $state('');
  let error = $state('');
  let logOpen = $state(false);
  let logLines = $state<string[]>([]);
  let logBox = $state<HTMLElement | null>(null);
  let timer: ReturnType<typeof setInterval> | null = null;

  const label = $derived(exitLabel(info.exit));
  const address = $derived(socksAddress(info.socks_port));
  const showBar = $derived(info.state === 'starting' || info.state === 'restarting');
  const errorText = $derived(info.error ? torErrorText(info.error, (k) => $t(k)) : '');

  async function act(run: () => Promise<unknown>, done = '') {
    busy = true;
    error = '';
    message = '';
    try {
      await run();
      message = done;
    } catch (e) {
      error = torErrorText(formatError(e), (k) => $t(k));
    } finally {
      busy = false;
    }
  }

  const newIdentity = () => act(() => api.tor.newIdentity(info.key), $t('tor_identity_done'));
  const stop = () => act(() => api.tor.stop(info.key));

  async function loadLog() {
    try {
      const lines = await api.tor.log(info.key);
      const atEnd = !logBox || logBox.scrollHeight - logBox.scrollTop - logBox.clientHeight < 24;
      logLines = lines;
      if (atEnd) queueMicrotask(() => { if (logBox) logBox.scrollTop = logBox.scrollHeight; });
    } catch {
      // The instance may have just gone; the next list drops the row.
    }
  }

  function stopPolling() {
    if (timer) clearInterval(timer);
    timer = null;
  }

  function toggleLog() {
    logOpen = !logOpen;
    stopPolling();
    if (logOpen) {
      loadLog();
      timer = setInterval(loadLog, LOG_POLL_MS);
    }
  }

  onDestroy(stopPolling);
</script>

<div class="instance">
  <div class="head">
    <span class="title">{label ?? $t('tor_exit_any')}</span>
    <span class="badge {STATE_BADGE[info.state]}">{$t(STATE_LABEL[info.state])}</span>
    {#if info.kept}<span class="badge">{$t('tor_kept')}</span>{/if}
    {#if info.restart_needed}<span class="badge badge-warn">{$t('tor_restart_needed')}</span>{/if}
  </div>

  {#if showBar}
    <div class="progress-wrap">
      <div class="progress-bar"><div class="progress-fill" style="width: {clampPercent(info.bootstrap)}%"></div></div>
      <span class="progress-label">{clampPercent(info.bootstrap)}%</span>
    </div>
  {/if}

  {#if info.summary}<p class="muted small">{info.summary}</p>{/if}

  <div class="facts">
    {#if address}
      <span class="fact"><span class="fact-label">SOCKS</span> <span class="mono">{address}</span></span>
    {/if}
    <span class="fact"><span class="fact-label">{$t('tor_consumers')}</span> {info.consumers}</span>
  </div>

  {#if info.restart_needed}<p class="warn-msg">{$t('tor_restart_needed_hint')}</p>{/if}
  {#if errorText}<div class="error-msg">{errorText}</div>{/if}
  {#if error}<div class="error-msg">{error}</div>{/if}
  {#if message}<p class="ok-msg">{message}</p>{/if}

  <div class="btn-row">
    <button class="btn btn-ghost btn-sm" disabled={busy || info.state !== 'ready'} onclick={newIdentity}>
      <Icon name="rotate-ccw" size={14} />
      {$t('tor_btn_new_identity')}
    </button>
    <button class="btn btn-ghost btn-sm" disabled={busy || info.state === 'stopping'} onclick={stop}>
      <Icon name="x" size={14} />
      {$t('tor_btn_stop')}
    </button>
    <button class="btn btn-ghost btn-sm" aria-expanded={logOpen} onclick={toggleLog}>
      <Icon name={logOpen ? 'chevron-down' : 'chevron-right'} size={14} />
      {logOpen ? $t('tor_btn_hide_log') : $t('tor_btn_show_log')}
    </button>
  </div>

  {#if logOpen}
    <pre class="log" bind:this={logBox} aria-label={$t('tor_log_label')}>{logLines.length ? logLines.join('\n') : $t('tor_log_empty')}</pre>
  {/if}
</div>

<style>
  .instance { display: flex; flex-direction: column; gap: var(--sp-2); padding: var(--sp-4); border: 1px solid var(--border); border-radius: var(--radius); }
  .head, .btn-row, .facts { display: flex; align-items: center; gap: var(--sp-2); flex-wrap: wrap; }
  .facts { gap: var(--sp-4); }
  .title { font-weight: var(--fw-bold); }
  .small { font-size: var(--fs-sm); }
  .fact { font-size: var(--fs-sm); color: var(--text-body); }
  .fact-label { color: var(--text-faint); margin-right: 0.25rem; }
  .mono { font-family: var(--font-mono); }
  .ok-msg { font-size: var(--fs-sm); color: var(--success-text); }
  .warn-msg { font-size: var(--fs-sm); color: var(--warn-text); }
  .error-msg { font-size: var(--fs-sm); color: var(--danger-text); }
  .progress-wrap { display: flex; flex-direction: column; gap: 0.3rem; }
  .progress-bar { height: 5px; background: var(--surface-2); border-radius: 999px; overflow: hidden; }
  .progress-fill { height: 100%; background: var(--accent); border-radius: 999px; transition: width 0.2s ease; }
  .progress-label { font-size: var(--fs-2xs); color: var(--text-2); font-family: var(--font-mono); }
  .log {
    margin: 0; max-height: 260px; overflow: auto; padding: var(--sp-2) var(--sp-3);
    background: var(--surface-3); border: 1px solid var(--border); border-radius: var(--radius);
    font-family: var(--font-mono); font-size: var(--fs-2xs); color: var(--text-body);
    white-space: pre-wrap; word-break: break-all;
  }
</style>
