<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- One Tor instance as a card: its exit and state with the marks of a hand start and a pending restart, the progress of its start, its address and users, and its actions and log under a line. -->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import { t, type TranslationKey } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { formatError } from '$lib/core/utils';
  import { api } from '$lib/tor/api';
  import { clampPercent } from '$lib/tor/format';
  import { exitLabel, socksAddress, torErrorText } from '$lib/tor/settings';
  import TorNote from '$lib/tor/desktop/TorNote.svelte';
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

<div class="card instance">
  <div class="head">
    <span class="title">{label ?? $t('tor_exit_any')}</span>
    <span class="badge {STATE_BADGE[info.state]}">{$t(STATE_LABEL[info.state])}</span>
    <span class="spacer"></span>
    {#if info.kept}<span class="badge badge-accent">{$t('tor_kept')}</span>{/if}
    {#if info.restart_needed}<span class="badge badge-warn">{$t('tor_restart_needed')}</span>{/if}
  </div>

  {#if showBar}
    <div class="bar" role="progressbar" aria-label={$t('settings_tor_title')} aria-valuemin={0} aria-valuemax={100} aria-valuenow={clampPercent(info.bootstrap)}>
      <div class="fill" style="width: {clampPercent(info.bootstrap)}%"></div>
    </div>
  {/if}

  <div class="facts">
    {#if showBar}<span class="mono">{clampPercent(info.bootstrap)}%</span>{/if}
    {#if info.summary}<span class="muted">{info.summary}</span>{/if}
    {#if address}<span><span class="muted">SOCKS</span> <span class="mono">{address}</span></span>{/if}
    <span><span class="muted">{$t('tor_consumers')}</span> {info.consumers}</span>
  </div>

  {#if info.restart_needed}<TorNote tone="warn">{$t('tor_restart_needed_hint')}</TorNote>{/if}
  {#if errorText}<TorNote tone="danger">{errorText}</TorNote>{/if}
  {#if error}<TorNote tone="danger">{error}</TorNote>{/if}
  {#if message}<TorNote tone="ok">{message}</TorNote>{/if}

  <div class="actions">
    <button class="btn btn-ghost" type="button" disabled={busy || info.state !== 'ready'} onclick={newIdentity}>
      <Icon name="refresh-cw" size={18} />
      {$t('tor_btn_new_identity')}
    </button>
    <button class="btn btn-ghost" type="button" disabled={busy || info.state === 'stopping'} onclick={stop}>
      <Icon name="x" size={18} />
      {$t('tor_btn_stop')}
    </button>
    <button class="btn btn-ghost quiet" type="button" aria-expanded={logOpen} onclick={toggleLog}>
      <Icon name={logOpen ? 'chevron-down' : 'chevron-right'} size={18} />
      {logOpen ? $t('tor_btn_hide_log') : $t('tor_btn_show_log')}
    </button>
  </div>

  {#if logOpen}
    <pre class="log mono" bind:this={logBox} aria-label={$t('tor_log_label')}>{logLines.length ? logLines.join('\n') : $t('tor_log_empty')}</pre>
  {/if}
</div>

<style>
  .instance { display: flex; flex-direction: column; gap: var(--sp-3); padding: var(--sp-4) var(--sp-5); }
  .head { display: flex; align-items: center; gap: var(--sp-2); flex-wrap: wrap; }
  .title { font-size: var(--fs-md); font-weight: var(--fw-bold); }
  .spacer { flex: 1; }
  .bar { height: 8px; border-radius: var(--radius-pill); background: var(--surface-2); overflow: hidden; }
  .fill { height: 100%; border-radius: var(--radius-pill); background: var(--accent); transition: width var(--dur-base); }
  .facts { display: flex; align-items: baseline; gap: var(--sp-5); flex-wrap: wrap; font-size: var(--fs-base); color: var(--text-body); }
  .actions { display: flex; flex-wrap: wrap; gap: var(--sp-2); padding-top: var(--sp-3); border-top: 1px solid var(--divider); }
  .quiet { color: var(--text-2); }
  .log {
    margin: 0; max-height: 260px; overflow: auto; padding: var(--sp-3) var(--sp-4);
    background: var(--surface-3); border: 1px solid var(--border); border-radius: var(--radius-md);
    font-size: var(--fs-xs); line-height: 1.6; color: var(--text-body);
    white-space: pre-wrap; overflow-wrap: anywhere;
  }
</style>
