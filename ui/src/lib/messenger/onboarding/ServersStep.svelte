<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { messengerStore } from '../store.svelte';
  import { messengerApi, messengerError, type MessengerManifestCheck } from '../api';
  import RelayAddForm from '../relays/RelayAddForm.svelte';
  import MediaServersPanel from '../media/MediaServersPanel.svelte';

  type Phase = 'choose' | 'veydan' | 'own';

  let phase = $state<Phase>('choose');
  let busy = $state(false);
  let error = $state('');
  let check = $state<MessengerManifestCheck | null>(null);

  const ownRelays = $derived(messengerStore.relays.filter((r) => r.source === 'user'));
  const canFinishOwn = $derived(ownRelays.some((r) => r.enabled));

  function explain(e: unknown): string {
    const m = messengerError(e);
    return m === 'servers_own_needs_relay' ? $t('msg_srv_own_needs_relay') : m;
  }

  function host(url: string): string {
    try { return new URL(url).host; } catch { return url; }
  }

  // The choice is made at once, the store is told when the user has read
  // the result: refreshing it ends the onboarding.
  async function chooseVeydan() {
    phase = 'veydan';
    error = '';
    busy = true;
    try { check = await messengerApi.relays.useVeydan(); }
    catch (e) { error = explain(e); }
    finally { busy = false; }
  }

  async function chooseOwn() {
    phase = 'own';
    error = '';
    await messengerStore.refreshRelays().catch(() => {});
  }

  async function finishOwn() {
    error = '';
    busy = true;
    try {
      await messengerApi.relays.useOwn();
      await messengerStore.refresh();
    } catch (e) { error = explain(e); }
    finally { busy = false; }
  }

  async function removeRelay(url: string) {
    error = '';
    try { await messengerStore.removeRelay(url); }
    catch (e) { error = explain(e); }
  }

  function back() {
    phase = 'choose';
    error = '';
    check = null;
  }
</script>

{#if phase === 'choose'}
  <div class="card-title">{$t('msg_srv_title')}</div>
  <p class="muted">{$t('msg_srv_text')}</p>
  <div class="options">
    <button class="option" onclick={chooseVeydan}>
      <span class="option-title"><Icon name="globe" size={16} />{$t('msg_srv_veydan_title')}</span>
      <span class="muted">{$t('msg_srv_veydan_text')}</span>
    </button>
    <button class="option" onclick={chooseOwn}>
      <span class="option-title"><Icon name="network" size={16} />{$t('msg_srv_own_title')}</span>
      <span class="muted">{$t('msg_srv_own_text')}</span>
    </button>
  </div>
{:else if phase === 'veydan'}
  <div class="card-title">{$t('msg_srv_veydan_title')}</div>
  {#if busy}
    <p class="muted"><span class="spinner"></span>{$t('msg_srv_fetching')}</p>
  {:else if check}
    <p class="result" class:fallback={!!check.error}>
      {#if check.error}
        {$t('msg_srv_fallback', { serial: String(check.serial) })}
      {:else}
        {$t('msg_srv_fetched', { serial: String(check.serial), host: host(check.origin) })}
      {/if}
    </p>
  {/if}
  {#if error}<div class="error-msg">{error}</div>{/if}
  <div class="actions">
    {#if error}
      <button class="btn btn-ghost" onclick={back}>{$t('msg_back')}</button>
      <button class="btn btn-primary" onclick={chooseVeydan}>{$t('msg_srv_veydan_title')}</button>
    {:else}
      <button class="btn btn-primary" disabled={busy || !check} onclick={() => messengerStore.refresh()}>{$t('msg_srv_done')}</button>
    {/if}
  </div>
{:else}
  <div class="card-title">{$t('msg_srv_own_title')}</div>
  <p class="muted">{$t('msg_srv_own_text')}</p>
  <div class="section-label">{$t('msg_srv_own_relays')}</div>
  {#if ownRelays.length}
    <ul class="list">
      {#each ownRelays as r (r.url)}
        <li class="row">
          <code>{r.url}</code>
          {#if r.auth_type === 'api_key'}<span class="meta">{$t('msg_relay_auth_api_key')}</span>{/if}
          <button class="icon-btn danger-soft" title={$t('msg_relay_remove')} disabled={busy} onclick={() => removeRelay(r.url)}>
            <Icon name="trash-2" size={13} />
          </button>
        </li>
      {/each}
    </ul>
  {:else}
    <p class="meta">{$t('msg_srv_own_empty')}</p>
  {/if}
  <RelayAddForm />
  <details class="media">
    <summary>{$t('msg_srv_own_media')}</summary>
    <MediaServersPanel />
  </details>
  <p class="meta">{$t('msg_srv_own_push')}</p>
  {#if error}<div class="error-msg">{error}</div>{/if}
  <div class="actions">
    <button class="btn btn-ghost" disabled={busy} onclick={back}>{$t('msg_back')}</button>
    <button class="btn btn-primary" disabled={busy || !canFinishOwn} onclick={finishOwn}>{$t('msg_srv_done')}</button>
  </div>
{/if}

<style>
  .muted { color: var(--text-2); font-size: var(--fs-sm); margin: 0; }
  .meta { color: var(--text-3); font-size: var(--fs-xs); margin: 0; }
  .options { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: var(--sp-2); }
  .option {
    display: flex; flex-direction: column; gap: var(--sp-1); text-align: left;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-sm);
    padding: var(--sp-3); cursor: pointer; color: var(--text); font: inherit;
  }
  .option:hover, .option:focus-visible { border-color: var(--accent-border); outline: none; }
  .option-title { display: flex; align-items: center; gap: var(--sp-2); font-weight: 600; }
  .result { margin: 0; font-size: var(--fs-sm); color: var(--success-text); }
  .result.fallback { color: var(--text-2); }
  .spinner {
    display: inline-block; width: 12px; height: 12px; margin-right: var(--sp-2); vertical-align: -1px;
    border: 2px solid var(--border); border-top-color: var(--accent-text); border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
  .section-label { color: var(--text-3); font-size: var(--fs-xs); text-transform: uppercase; letter-spacing: 0.5px; }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-1); }
  .row {
    display: flex; align-items: center; gap: var(--sp-2);
    padding: var(--sp-2) var(--sp-3); border: 1px solid var(--border); border-radius: var(--radius-sm);
  }
  .row code { flex: 1; min-width: 0; font-family: var(--font-mono); font-size: var(--fs-xs); word-break: break-all; }
  .media summary { cursor: pointer; font-size: var(--fs-sm); color: var(--text-2); margin-bottom: var(--sp-2); }
  .actions { display: flex; gap: var(--sp-2); flex-wrap: wrap; justify-content: flex-end; }
</style>
