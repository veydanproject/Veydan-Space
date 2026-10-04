<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  Whether the project's servers are reached directly or through a bridge,
  and the bridges the user added. With the user's own servers chosen there
  is nothing to set: a bridge carries only the project's servers.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import type { NetMode } from '../api';
  import { netErrorText } from './errors';
  import { netStore } from './netStore.svelte';

  const MODES: NetMode[] = ['off', 'on', 'auto'];
  let newBridge = $state('');

  onMount(() => { netStore.load(); });

  const s = $derived(netStore.status);
  const error = $derived(netErrorText(netStore.error, $t));
  const way = $derived.by(() => {
    if (!s) return '';
    if (!s.active) return $t('msg_bridge_way_direct');
    return s.bridge ? $t('msg_bridge_way_via', { bridge: s.bridge }) : $t('msg_bridge_way_connecting');
  });

  async function add() {
    const text = newBridge.trim();
    if (!text) return;
    if (await netStore.addBridge(text)) newBridge = '';
  }
</script>

<div class="card bridges">
  <div class="card-title"><Icon name="shield" size={16} />{$t('msg_bridge_title')}</div>
  <p class="muted">{$t('msg_bridge_intro')}</p>

  {#if s && !s.available}
    <p class="muted">{$t('msg_bridge_own_servers')}</p>
  {:else if s}
    <div class="seg" role="radiogroup" aria-label={$t('msg_bridge_title')}>
      {#each MODES as mode}
        <button
          class="seg-btn" class:active={s.mode === mode} role="radio" aria-checked={s.mode === mode}
          disabled={netStore.busy} onclick={() => netStore.setMode(mode)}
        >{$t(`msg_bridge_mode_${mode}` as 'msg_bridge_mode_off')}</button>
      {/each}
    </div>
    <p class="hint">{$t(`msg_bridge_mode_${s.mode}_hint` as 'msg_bridge_mode_off_hint')}</p>

    <div class="state">
      <span class="dot" class:via={s.active}></span>
      <span class="way">{way}</span>
      <span class="known">{$t('msg_bridge_known', { count: String(s.bridges) })}</span>
      <button class="btn btn-ghost btn-sm" disabled={netStore.busy} onclick={() => netStore.runCheck()}>
        <Icon name="refresh-cw" size={12} />{$t('msg_bridge_check')}
      </button>
    </div>
    {#if netStore.check}
      <div class="meta">{$t(`msg_bridge_verdict_${netStore.check.verdict}` as 'msg_bridge_verdict_direct')}</div>
    {/if}

    {#if s.private.length}
      <ul class="list">
        {#each s.private as bridge (bridge.id)}
          <li class="row">
            <code>{bridge.addr}</code>
            <span class="meta">{bridge.id.slice(0, 12)}…</span>
            <span class="spacer"></span>
            <button
              class="icon-btn danger-soft" disabled={netStore.busy} title={$t('msg_bridge_remove')}
              aria-label={$t('msg_bridge_remove')} onclick={() => netStore.removeBridge(bridge.id)}
            ><Icon name="trash-2" size={14} /></button>
          </li>
        {/each}
      </ul>
    {/if}

    <form class="add" onsubmit={(e) => { e.preventDefault(); add(); }}>
      <input type="text" bind:value={newBridge} placeholder="veydan://vlink/…" spellcheck="false" disabled={netStore.busy} />
      <button class="btn btn-ghost" type="submit" disabled={netStore.busy || !newBridge.trim()}>
        <Icon name="plus" size={14} />{$t('msg_bridge_add')}
      </button>
    </form>
    <p class="hint">{$t('msg_bridge_add_hint')}</p>
  {/if}
  {#if error}<div class="error-msg">{error}</div>{/if}
</div>

<style>
  .card-title { display: flex; align-items: center; gap: var(--sp-2); }
  .bridges { display: flex; flex-direction: column; gap: var(--sp-3); }
  .muted, .hint { margin: 0; font-size: var(--fs-sm); color: var(--text-2); line-height: 1.45; }
  .hint { font-size: var(--fs-xs); color: var(--text-3); }
  .seg { align-self: flex-start; }
  .state { display: flex; align-items: center; gap: var(--sp-2); flex-wrap: wrap; font-size: var(--fs-sm); }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--text-3); flex-shrink: 0; }
  .dot.via { background: var(--success-text); }
  .way { color: var(--text); }
  .known { color: var(--text-3); font-size: var(--fs-xs); margin-right: auto; }
  .meta { font-size: var(--fs-xs); color: var(--text-3); }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-1); }
  .row { display: flex; align-items: center; gap: var(--sp-2); min-width: 0; }
  .row code { font-family: var(--font-mono); font-size: var(--fs-xs); }
  .spacer { flex: 1; }
  .add { display: flex; gap: var(--sp-2); flex-wrap: wrap; }
  .add input {
    flex: 1; min-width: 0; font: inherit; font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text);
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 8px 10px;
  }
  .add input:focus { outline: none; border-color: var(--accent-border); }
</style>
