<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The call nodes, in the network settings: which nodes calls may use (any,
  the project's and mine, mine only), whether a call always goes through a
  relay, and every node the sets know in the order a call tries them, with
  whose it is, where it is, how near and how full. A private node is added
  by the link its owner gives (an invitation, spent once: asked first), or
  by its reference or a link without an invitation, with a key given by
  hand; only mine can be removed.

  The list shows what the runtime knows; on opening, the nodes a call may
  use are asked once (their round trip and state). Under "mine only" no
  other node is asked, and the registry of volunteers is asked only under
  "any", with the Veydan servers and the silent mode off (otherwise the
  panel says it is paused).
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { ask } from '$lib/core/ui/confirm.svelte';
  import type { CallNodeInfo, CallTrust } from '../api';
  import { callStore } from '../calls/callStore.svelte';
  import { stamp } from '../shared/time';
  import { callNodeErrorText } from './errors';
  import { callNodesStore, carriesInvitation, linkAddress, takesKey } from './callNodesStore.svelte';

  const LEVELS: CallTrust[] = ['any', 'project_and_own', 'own_only'];
  const tr = (key: string, params?: Record<string, string>) => $t(key as 'msg_title', params);

  let text = $state('');
  let key = $state('');

  onMount(() => {
    callNodesStore.clearError();
    void callNodesStore.load();
    if (!callStore.loaded) callStore.load().catch(() => {});
  });

  const v = $derived(callNodesStore.view);
  const error = $derived(callNodeErrorText(callNodesStore.error, (k) => tr(k)));
  /** A reference, or a link without an invitation: its key is given by hand. */
  const keyed = $derived(takesKey(text));
  const relayOnly = $derived(callStore.policy === 'relay_only');

  function classWord(n: CallNodeInfo): string {
    if (n.class === 'own') return $t('msg_calls_node_own');
    if (n.class === 'volunteer') return $t('msg_calls_node_volunteer');
    if (n.class === 'cloud') return $t('msg_calls_node_cloud');
    return $t('msg_calls_node_project');
  }

  function short(id: string): string {
    return `${id.slice(0, 8)}…${id.slice(-4)}`;
  }

  async function add() {
    const link = text.trim();
    if (!link) return;
    if (carriesInvitation(link)) {
      const yes = await ask({
        title: $t('msg_call_nodes_confirm_title'),
        message: $t('msg_call_nodes_confirm_text', { addr: linkAddress(link) }),
        confirmLabel: $t('msg_call_nodes_confirm_yes'),
        variant: 'primary',
      });
      if (!yes) return;
    }
    if ((await callNodesStore.add(link, keyed ? key : undefined)) === '') {
      text = '';
      key = '';
    }
  }

  async function remove(n: CallNodeInfo) {
    const yes = await ask({
      title: $t('msg_call_nodes_remove_title'),
      message: $t('msg_call_nodes_remove_text', { addr: n.addr }),
      confirmLabel: $t('msg_calls_node_remove'),
    });
    if (yes) await callNodesStore.remove(n.id);
  }
</script>

<div class="card nodes">
  <div class="card-title"><Icon name="network" size={16} />{$t('msg_calls_nodes_title')}</div>
  <p class="muted">{$t('msg_calls_nodes_intro')}</p>

  {#if v}
    <div class="block">
      <div class="label">{$t('msg_call_nodes_trust')}</div>
      <div class="seg" role="radiogroup" aria-label={$t('msg_call_nodes_trust')}>
        {#each LEVELS as level}
          <button class="seg-btn" class:active={v.trust === level} role="radio" aria-checked={v.trust === level}
            disabled={callNodesStore.busy} onclick={() => callNodesStore.setTrust(level)}>
            {tr(`msg_call_nodes_trust_${level}`)}
          </button>
        {/each}
      </div>
      <p class="hint">{tr(`msg_call_nodes_trust_${v.trust}_hint`)}</p>
    </div>
  {/if}

  <div class="block">
    <div class="line">
      <span>{$t('msg_calls_policy_relay_only')}</span>
      <button class="toggle" class:on={relayOnly} disabled={callStore.busy || !callStore.loaded}
        onclick={() => callStore.setPolicy(relayOnly ? 'auto' : 'relay_only')}
        aria-pressed={relayOnly} aria-label={$t('msg_calls_policy_relay_only')}></button>
    </div>
    <p class="hint">{$t(relayOnly ? 'msg_calls_policy_relay_only_hint' : 'msg_calls_policy_auto_hint')}</p>
  </div>

  {#if v}
    <div class="block">
      <div class="line">
        <span class="label">{$t('msg_call_nodes_list')}</span>
        <button class="btn btn-ghost btn-sm" disabled={callNodesStore.probing || callNodesStore.busy} onclick={() => callNodesStore.probe()}>
          <span class:spin={callNodesStore.probing}><Icon name="refresh-cw" size={12} /></span>{$t('msg_call_nodes_check')}
        </button>
      </div>
      {#if v.nodes.length}
        <ul class="list">
          {#each v.nodes as n (n.id)}
            <li class="row" class:unused={!n.used} title={n.used ? n.reference : `${$t('msg_call_nodes_unused')} · ${n.reference}`}>
              <span class="dot {n.health}" aria-hidden="true"></span>
              <div class="node">
                <div class="top">
                  <code>{n.addr}</code>
                  {#if n.label}<span class="name">{n.label}</span>{/if}
                </div>
                <div class="meta">
                  <span class="tag" class:own={n.mine} class:volunteer={n.class === 'volunteer'}>{classWord(n)}</span>
                  {#if n.region}<span>{n.region.toUpperCase()}</span>{/if}
                  <span>{tr(`msg_call_nodes_state_${n.health}`)}</span>
                  {#if n.rtt_ms != null}<span>{$t('msg_call_nodes_rtt', { ms: String(n.rtt_ms) })}</span>{/if}
                  {#if n.load != null}<span>{$t('msg_call_nodes_load', { n: String(n.load) })}</span>{/if}
                  {#if n.private}<span>{$t('msg_call_nodes_private')}</span>{/if}
                  {#if n.has_key}<span class="key" title={$t('msg_calls_node_key')}><Icon name="key" size={10} /></span>{/if}
                  {#if !n.used}<span>{$t('msg_call_nodes_unused')}</span>{/if}
                  <span class="id">#{short(n.id)}</span>
                </div>
              </div>
              {#if n.mine}
                <button class="icon-btn danger-soft" disabled={callNodesStore.busy} title={$t('msg_calls_node_remove')}
                  aria-label={$t('msg_calls_node_remove')} onclick={() => remove(n)}><Icon name="trash-2" size={14} /></button>
              {/if}
            </li>
          {/each}
        </ul>
      {:else}
        <p class="hint">{$t('msg_calls_nodes_empty')}</p>
      {/if}
      {#if v.registry && v.trust === 'any'}
        <div class="registry">
          <span>{v.registry_checked_at ? $t('msg_call_nodes_registry', { time: stamp(v.registry_checked_at) }) : $t('msg_call_nodes_registry_never')}</span>
          {#if !v.registry_paused}
            <button class="btn btn-ghost btn-sm" disabled={callNodesStore.busy} onclick={() => callNodesStore.refreshRegistry()}>
              {$t('msg_call_nodes_registry_refresh')}
            </button>
          {/if}
        </div>
        {#if v.registry_paused}<p class="hint">{$t('msg_call_nodes_registry_paused')}</p>{/if}
      {/if}
    </div>
  {/if}

  <div class="block">
    <div class="label">{$t('msg_call_nodes_add_title')}</div>
    <form class="add" onsubmit={(e) => { e.preventDefault(); add(); }}>
      <input class="ref" type="text" bind:value={text} placeholder={$t('msg_call_nodes_add_placeholder')} spellcheck="false"
        autocomplete="off" autocapitalize="off" disabled={callNodesStore.busy} aria-label={$t('msg_call_nodes_add_title')} />
      {#if keyed}
        <input class="key" type="password" bind:value={key} placeholder={$t('msg_calls_node_key_placeholder')} autocomplete="off" disabled={callNodesStore.busy} />
      {/if}
      <button class="btn btn-ghost" type="submit" disabled={callNodesStore.busy || !text.trim()}>
        <Icon name="plus" size={14} />{$t('msg_calls_node_add')}
      </button>
    </form>
    <p class="hint">{$t('msg_call_nodes_add_hint')}</p>
  </div>
  {#if error}<div class="error-msg">{error}</div>{/if}
</div>

<style>
  .nodes { display: flex; flex-direction: column; gap: var(--sp-3); }
  .card-title { display: flex; align-items: center; gap: var(--sp-2); }
  .muted, .hint { margin: 0; font-size: var(--fs-sm); color: var(--text-2); line-height: 1.45; }
  .hint { font-size: var(--fs-xs); color: var(--text-3); }
  .block { display: flex; flex-direction: column; gap: var(--sp-2); }
  .label { font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text); }
  .line { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-3); font-size: var(--fs-sm); color: var(--text); }
  /* A phone is narrower than three choices in one line: each wraps its words instead of the row running off. */
  .seg { align-self: flex-start; max-width: 100%; }
  .seg-btn { flex: 1 1 auto; min-width: 0; height: auto; min-height: 34px; padding-block: 6px; white-space: normal; text-align: center; justify-content: center; line-height: 1.25; }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .row { display: flex; align-items: center; gap: var(--sp-2); min-width: 0; padding: 7px 0; border-bottom: 1px solid var(--border); }
  .row:last-child { border-bottom: none; }
  .row.unused .node { opacity: 0.55; }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--text-3); flex-shrink: 0; }
  .dot.active { background: var(--success-text); }
  .dot.degraded { background: var(--warn-text); }
  .dot.refused, .dot.unreachable { background: var(--danger-text); }
  .node { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
  .top { display: flex; align-items: baseline; gap: var(--sp-2); min-width: 0; }
  .top code { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text); white-space: nowrap; }
  .name { font-size: var(--fs-xs); color: var(--text-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .meta { display: flex; align-items: center; flex-wrap: wrap; gap: 2px 8px; font-size: var(--fs-2xs); color: var(--text-3); }
  .meta .id { font-family: var(--font-mono); }
  .meta .key { display: inline-flex; }
  .tag {
    display: inline-flex; align-items: center; padding: 0 7px; border-radius: var(--radius-pill);
    background: var(--surface-2); border: 1px solid var(--border); color: var(--text-2); white-space: nowrap;
  }
  .tag.own { background: var(--accent-tint); border-color: var(--accent-tint-border); color: var(--accent-text-2); }
  .tag.volunteer { border-style: dashed; }
  .registry { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); flex-wrap: wrap; font-size: var(--fs-xs); color: var(--text-3); }
  .add { display: flex; gap: var(--sp-2); flex-wrap: wrap; }
  .add input {
    min-width: 0; font: inherit; font-size: var(--fs-xs); color: var(--text);
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 8px 10px;
  }
  .add .ref { flex: 2 1 220px; font-family: var(--font-mono); }
  .add .key { flex: 1 1 140px; }
  .add input:focus { outline: none; border-color: var(--accent-border); }
  .spin { display: inline-flex; animation: spin 1s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .spin { animation: none; } }
</style>
