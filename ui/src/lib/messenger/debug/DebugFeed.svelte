<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import { clockSeconds } from '../shared/time';
  import Icon from '$lib/core/Icon.svelte';
  import { messengerStore } from '../store.svelte';
  import { messengerError } from '../api';

  let to = $state('');
  let text = $state('');
  let busy = $state(false);
  let error = $state('');
  let lastSent = $state('');

  onMount(() => { messengerStore.startListeners().catch(() => {}); });

  const canSend = $derived(!!messengerStore.status?.runtime?.session_active);

  async function send() {
    error = ''; lastSent = '';
    if (!to.trim() || !text.trim()) return;
    busy = true;
    try {
      lastSent = await messengerStore.sendTextDm(to.trim(), text.trim());
      text = '';
    } catch (e) { error = messengerError(e); }
    finally { busy = false; }
  }

  function short(v: unknown): string {
    const s = typeof v === 'string' ? v : '';
    return s.length > 16 ? `${s.slice(0, 8)}…${s.slice(-4)}` : s;
  }

  function describe(name: string, payload: unknown): string {
    const p = (payload ?? {}) as Record<string, unknown>;
    switch (name) {
      case 'inbound.dm':
        return `${short(p.sender)}: ${typeof p.text === 'string' ? p.text : `[${String(p.type)}]`}${p.historical ? ' (hist)' : ''}`;
      case 'inbound.meta':
        return `${String(p.what)} ${short(p.author)}`;
      case 'ignored':
        return `${p.kind !== undefined ? `kind ${String(p.kind)}` : String(p.family)} — ${String(p.reason)}`;
      case 'error':
        return `${String(p.family)}: ${String(p.error)}`;
      default:
        return JSON.stringify(payload);
    }
  }
</script>

<div class="card feed">
  <div class="card-title"><Icon name="zap" size={16} /> {$t('msg_debug_title')}</div>
  <p class="muted">{$t('msg_debug_text')}</p>

  <form class="send" onsubmit={(e) => { e.preventDefault(); send(); }}>
    <input type="text" bind:value={to} placeholder="npub1… / hex" spellcheck="false" disabled={busy || !canSend} />
    <input type="text" bind:value={text} placeholder={$t('msg_debug_send_placeholder')} disabled={busy || !canSend} />
    <button class="btn btn-primary" type="submit" disabled={busy || !canSend || !to.trim() || !text.trim()}>
      <Icon name="message-circle" size={14} />{$t('msg_debug_send')}
    </button>
  </form>
  {#if !canSend}<div class="muted small">{$t('msg_debug_no_session')}</div>{/if}
  {#if lastSent}<div class="muted small">{$t('msg_debug_sent', { id: lastSent })}</div>{/if}
  {#if error}<div class="error-msg">{error}</div>{/if}

  <div class="feed-head">
    <span class="muted small">{$t('msg_debug_feed', { n: String(messengerStore.feed.length) })}</span>
    <button class="btn btn-ghost btn-sm" disabled={!messengerStore.feed.length} onclick={() => messengerStore.clearFeed()}>{$t('msg_debug_clear')}</button>
  </div>
  <ul class="list">
    {#each messengerStore.feed as e, i (e.at + ':' + i)}
      <li class="row {e.name.replace('.', '-')}">
        <span class="time">{clockSeconds(e.at)}</span>
        <span class="name">{e.name}</span>
        <span class="body">{describe(e.name, e.payload)}</span>
      </li>
    {/each}
  </ul>
</div>

<style>
  .feed { max-width: 640px; display: flex; flex-direction: column; gap: var(--sp-3); }
  .card-title { display: flex; align-items: center; gap: var(--sp-2); }
  .muted { color: var(--text-2); font-size: var(--fs-sm); margin: 0; }
  .small { font-size: var(--fs-xs); }
  .send { display: flex; gap: var(--sp-2); flex-wrap: wrap; }
  .send input {
    flex: 1; min-width: 160px; font: inherit; font-size: var(--fs-sm); color: var(--text);
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 8px 10px;
  }
  .send input:first-child { font-family: var(--font-mono); font-size: var(--fs-xs); }
  .send input:focus { outline: none; border-color: var(--accent-border); }
  .feed-head { display: flex; align-items: center; justify-content: space-between; }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; max-height: 320px; overflow-y: auto; }
  .row {
    display: grid; grid-template-columns: 70px 100px 1fr; gap: var(--sp-2); align-items: baseline;
    font-size: var(--fs-xs); padding: 4px 8px; border-radius: var(--radius-sm); background: var(--surface-2);
  }
  .time { color: var(--text-3); font-family: var(--font-mono); }
  .name { font-family: var(--font-mono); color: var(--text-2); }
  .row.inbound-dm .name { color: var(--success-text); }
  .row.error .name { color: var(--danger-text); }
  /* A payload is data: monospace, broken anywhere rather than run out of the row. */
  .body { color: var(--text-body); font-family: var(--font-mono); overflow-wrap: anywhere; min-width: 0; }
</style>
