<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import { stamp } from '../shared/time';
  import Icon from '$lib/core/Icon.svelte';
  import { messengerStore } from '../store.svelte';
  import { messengerError, type RelayState } from '../api';
  import RelayAddForm from './RelayAddForm.svelte';

  let busy = $state(false);
  let error = $state('');
  let notice = $state('');

  onMount(() => { messengerStore.startListeners().catch(() => {}); });

  const stateLabel: Record<RelayState, string> = {
    connected: 'msg_relay_state_connected',
    connecting: 'msg_relay_state_connecting',
    disconnected: 'msg_relay_state_disconnected',
    paused: 'msg_relay_state_paused',
  };

  async function run(fn: () => Promise<unknown>) {
    error = '';
    busy = true;
    try { await fn(); }
    catch (e) {
      const m = messengerError(e);
      error = m === 'servers_own_needs_relay' ? $t('msg_srv_own_needs_relay') : m;
    }
    finally { busy = false; }
  }

  const mode = $derived(messengerStore.manifest?.mode ?? null);

  function originLabel(origin: string): string {
    if (origin === 'embedded') return $t('msg_srv_origin_embedded');
    try { return new URL(origin).host; } catch { return origin; }
  }

  async function checkManifest() {
    notice = '';
    await run(async () => {
      const c = await messengerStore.refreshManifest();
      if (!c) return;
      notice = c.error
        ? $t('msg_srv_check_failed', { error: c.error })
        : c.updated ? $t('msg_srv_check_updated', { serial: String(c.serial) }) : $t('msg_srv_check_none');
    });
  }

  function switchMode() {
    notice = '';
    run(() => (mode === 'own' ? messengerStore.useVeydanServers() : messengerStore.useOwnServers()));
  }
</script>

<div class="card relays">
  <div class="card-title"><Icon name="globe" size={16} /> {$t('msg_relays_title')}</div>
  <p class="muted">{$t('msg_relays_text')}</p>

  {#if messengerStore.manifest}
    <div class="servers">
      <div class="servers-info">
        <span>{$t('msg_srv_mode')}: <b>{mode === 'own' ? $t('msg_srv_mode_own') : $t('msg_srv_mode_veydan')}</b></span>
        {#if mode === 'veydan'}
          <span class="meta">
            {$t('msg_srv_list', {
              serial: String(messengerStore.manifest.serial ?? '—'),
              origin: originLabel(messengerStore.manifest.origin),
              checked: messengerStore.manifest.checked_at ? stamp(messengerStore.manifest.checked_at) : $t('msg_srv_never'),
            })}
          </span>
        {/if}
      </div>
      <div class="servers-actions">
        {#if mode === 'veydan'}
          <button class="btn btn-ghost btn-sm" disabled={busy} onclick={checkManifest}>
            <Icon name="refresh-cw" size={13} />{$t('msg_srv_check')}
          </button>
        {/if}
        <button class="btn btn-ghost btn-sm" disabled={busy} onclick={switchMode}>
          {mode === 'own' ? $t('msg_srv_use_veydan') : $t('msg_srv_use_own')}
        </button>
      </div>
    </div>
    {#if notice}<div class="meta">{notice}</div>{/if}
    <div class="controls">
      {#if mode === 'veydan'}
      <label class="control">
        <span>{$t('msg_relays_region')}</span>
        <select
          value={messengerStore.manifest.region}
          disabled={busy}
          onchange={(e) => run(() => messengerStore.setRegion((e.currentTarget as HTMLSelectElement).value))}
        >
          {#each messengerStore.manifest.regions as r}
            <option value={r}>{r}</option>
          {/each}
          {#if !messengerStore.manifest.regions.includes(messengerStore.manifest.region)}
            <option value={messengerStore.manifest.region}>{messengerStore.manifest.region}</option>
          {/if}
        </select>
      </label>
      {/if}
      <div class="control silent">
        <div class="control-info">
          <span>{$t('msg_relays_silent')}</span>
          <span class="muted small">{$t('msg_relays_silent_hint')}</span>
        </div>
        <button
          class="toggle"
          class:on={messengerStore.manifest.silent_mode}
          disabled={busy}
          onclick={() => run(() => messengerStore.setSilent(!messengerStore.manifest?.silent_mode))}
          aria-pressed={messengerStore.manifest.silent_mode}
          aria-label={$t('msg_relays_silent')}
        ></button>
      </div>
    </div>
  {/if}

  <ul class="list">
    {#each messengerStore.relays as r (r.url)}
      <li class="row" class:off={!r.enabled}>
        <span class="dot {r.state}" title={$t(stateLabel[r.state] as 'msg_relay_state_connected')}></span>
        <div class="info">
          <code>{r.url}</code>
          <span class="meta">
            {r.source === 'manifest' ? $t('msg_relay_source_manifest') : $t('msg_relay_source_user')}
            {#if r.relay_id} · {r.relay_id}{/if}
            {#if r.auth_type === 'api_key'} · {$t('msg_relay_auth_api_key')}{:else if r.auth_type === 'nip42'} · NIP-42{/if}
            · {$t(stateLabel[r.state] as 'msg_relay_state_connected')}
          </span>
        </div>
        <button
          class="toggle"
          class:on={r.enabled}
          disabled={busy}
          onclick={() => run(() => messengerStore.setRelayEnabled(r.url, !r.enabled))}
          aria-pressed={r.enabled}
          aria-label={$t('msg_relay_enabled')}
        ></button>
        {#if r.source === 'user'}
          <button class="icon-btn danger-soft" title={$t('msg_relay_remove')} disabled={busy} onclick={() => run(() => messengerStore.removeRelay(r.url))}>
            <Icon name="trash-2" size={13} />
          </button>
        {:else}
          <span class="icon-spacer"></span>
        {/if}
      </li>
    {/each}
  </ul>

  <RelayAddForm />
  {#if error}<div class="error-msg">{error}</div>{/if}
</div>

<style>
  .relays { max-width: 640px; display: flex; flex-direction: column; gap: var(--sp-3); }
  .card-title { display: flex; align-items: center; gap: var(--sp-2); }
  .muted { color: var(--text-2); font-size: var(--fs-sm); margin: 0; }
  .small { font-size: var(--fs-xs); }
  .meta { color: var(--text-3); font-size: var(--fs-xs); }
  .controls { display: flex; flex-wrap: wrap; align-items: center; gap: var(--sp-3) var(--sp-4); }
  .control { display: flex; align-items: center; gap: var(--sp-2); font-size: var(--fs-sm); }
  .control select {
    font: inherit; color: var(--text); background: var(--surface-2);
    border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 5px 8px;
  }
  .control-info { display: flex; flex-direction: column; }
  /* A setting row like every other switch: the text takes the row, the switch sits at its end. */
  .control.silent { flex: 1 1 100%; gap: var(--sp-3); }
  .control.silent .control-info { flex: 1; min-width: 0; }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-1); }
  .row {
    display: flex; align-items: center; gap: var(--sp-3);
    padding: var(--sp-2) var(--sp-3); border: 1px solid var(--border); border-radius: var(--radius-sm);
    background: var(--surface);
  }
  .row.off { opacity: 0.6; }
  .info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  code { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-body); word-break: break-all; }
  .dot { width: 9px; height: 9px; border-radius: 50%; flex-shrink: 0; background: var(--text-3); }
  .dot.connected { background: var(--success-text); }
  .dot.connecting { background: var(--color-warning); }
  .dot.paused { background: var(--danger-text); }
  .icon-spacer { width: 28px; }
  .servers { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: var(--sp-2) var(--sp-3); font-size: var(--fs-sm); }
  .servers-info { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .servers-actions { display: flex; gap: var(--sp-2); flex-wrap: wrap; }
</style>
