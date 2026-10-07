<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  The settings of calls: which way a call may take (directly when it
  works, or always through a relay, which keeps my address from the
  peer), the quality of my video, and the call nodes: the project's, and the developer's own ones
  (`address:port#id`, with the key of a private node), tried first.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import type { RelayPolicy, VideoQuality } from '../api';
  import CallIcon from './CallIcon.svelte';
  import { callStore } from './callStore.svelte';
  import { callErrorText } from './words';

  const POLICIES: RelayPolicy[] = ['auto', 'relay_only'];
  const QUALITIES: VideoQuality[] = ['360p', '720p'];

  const tr = (key: string, params?: Record<string, string>) => $t(key as 'msg_title', params);

  let reference = $state('');
  let key = $state('');
  let refusal = $state('');

  onMount(() => {
    // A refusal left from a call's screen is not about these settings.
    if (!callStore.call) callStore.clearError();
    callStore.load().catch((e) => (refusal = callErrorText(e, tr)));
  });

  const error = $derived(refusal || (callStore.error && !callStore.call ? callErrorText(callStore.error, tr) : ''));
  const shape = /^[^\s#]+:\d{1,5}#[0-9a-fA-F]{64}$/;

  async function add() {
    refusal = '';
    const ref = reference.trim();
    if (!shape.test(ref)) { refusal = $t('msg_calls_node_invalid'); return; }
    if (await callStore.addNode(ref, key)) { reference = ''; key = ''; }
  }

  function short(id: string): string {
    return `${id.slice(0, 10)}…${id.slice(-4)}`;
  }
</script>

<div class="card calls">
  <div class="card-title"><CallIcon name="phone" size={16} />{$t('msg_calls_title')}</div>
  <p class="muted">{$t('msg_calls_intro')}</p>
  {#if callStore.loaded && !callStore.available}
    <p class="warn-line">{$t('msg_calls_unavailable')}</p>
  {/if}

  <div class="block">
    <div class="label">{$t('msg_calls_policy')}</div>
    <div class="seg" role="radiogroup" aria-label={$t('msg_calls_policy')}>
      {#each POLICIES as p}
        <button class="seg-btn" class:active={callStore.policy === p} role="radio" aria-checked={callStore.policy === p}
          disabled={callStore.busy || !callStore.loaded} onclick={() => { refusal = ''; callStore.setPolicy(p); }}>
          {$t(`msg_calls_policy_${p}` as 'msg_calls_policy_auto')}
        </button>
      {/each}
    </div>
    <p class="hint">{$t(`msg_calls_policy_${callStore.policy}_hint` as 'msg_calls_policy_auto_hint')}</p>
  </div>
</div>

<div class="card calls">
  <div class="card-title"><CallIcon name="video" size={16} />{$t('msg_calls_video_title')}</div>
  <div class="block">
    <div class="label">{$t('msg_calls_video_quality')}</div>
    <div class="seg" role="radiogroup" aria-label={$t('msg_calls_video_quality')}>
      {#each QUALITIES as q}
        <button class="seg-btn" class:active={callStore.videoQuality === q} role="radio" aria-checked={callStore.videoQuality === q}
          disabled={callStore.busy || !callStore.loaded} onclick={() => { refusal = ''; callStore.setVideoQuality(q); }}>
          {$t(`msg_calls_video_${q}` as 'msg_calls_video_360p')}
        </button>
      {/each}
    </div>
    <p class="hint">{$t('msg_calls_video_hint')}</p>
  </div>
</div>

<div class="card calls">
  <div class="card-title"><Icon name="network" size={16} />{$t('msg_calls_nodes_title')}</div>
  <p class="muted">{$t('msg_calls_nodes_intro')}</p>

  {#if callStore.nodes.length}
    <ul class="list">
      {#each callStore.nodes as node (node.reference)}
        <li class="row">
          <div class="node">
            <code title={node.reference}>{node.reference.split('#')[0]}</code>
            <span class="meta">#{short(node.id)}</span>
          </div>
          <span class="spacer"></span>
          {#if node.has_key}<span class="tag" title={$t('msg_calls_node_key')}><Icon name="key" size={11} /></span>{/if}
          <span class="tag" class:own={node.class === 'own'}>{$t(node.class === 'own' ? 'msg_calls_node_own' : 'msg_calls_node_project')}</span>
          {#if node.class === 'own'}
            <button class="icon-btn danger-soft" disabled={callStore.busy} title={$t('msg_calls_node_remove')} aria-label={$t('msg_calls_node_remove')}
              onclick={() => { refusal = ''; callStore.removeNode(node.reference); }}><Icon name="trash-2" size={14} /></button>
          {/if}
        </li>
      {/each}
    </ul>
  {:else if callStore.loaded}
    <p class="hint">{$t('msg_calls_nodes_empty')}</p>
  {/if}

  <form class="add" onsubmit={(e) => { e.preventDefault(); add(); }}>
    <input class="ref" type="text" bind:value={reference} placeholder="203.0.113.7:8443#…" spellcheck="false" autocomplete="off" disabled={callStore.busy} aria-label={$t('msg_calls_nodes_title')} />
    <input class="key" type="password" bind:value={key} placeholder={$t('msg_calls_node_key_placeholder')} autocomplete="off" disabled={callStore.busy} />
    <button class="btn btn-ghost" type="submit" disabled={callStore.busy || !reference.trim()}>
      <Icon name="plus" size={14} />{$t('msg_calls_node_add')}
    </button>
  </form>
  {#if error}<div class="error-msg">{error}</div>{/if}
</div>

<style>
  .calls { display: flex; flex-direction: column; gap: var(--sp-3); }
  .card-title { display: flex; align-items: center; gap: var(--sp-2); }
  .muted, .hint { margin: 0; font-size: var(--fs-sm); color: var(--text-2); line-height: 1.45; }
  .hint { font-size: var(--fs-xs); color: var(--text-3); }
  .warn-line { margin: 0; font-size: var(--fs-sm); color: var(--warn-text); background: var(--warn-bg); border: 1px solid var(--warn-border); border-radius: var(--radius-sm); padding: 8px 10px; }
  .block { display: flex; flex-direction: column; gap: var(--sp-2); }
  .label { font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--text); }
  /* A phone is narrower than the two choices in one line: each wraps its words instead of the row running off. */
  .seg { align-self: flex-start; max-width: 100%; }
  .seg-btn { flex: 1 1 auto; min-width: 0; height: auto; min-height: 34px; padding-block: 6px; white-space: normal; text-align: center; justify-content: center; line-height: 1.25; }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--sp-1); }
  .row { display: flex; align-items: center; gap: var(--sp-2); min-width: 0; padding: 6px 0; border-bottom: 1px solid var(--border); }
  .row:last-child { border-bottom: none; }
  .node { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
  .node code { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .meta { font-family: var(--font-mono); font-size: var(--fs-2xs); color: var(--text-3); }
  .spacer { flex: 1; }
  .tag {
    display: inline-flex; align-items: center; gap: 3px; padding: 1px 8px; border-radius: var(--radius-pill);
    font-size: var(--fs-2xs); background: var(--surface-2); border: 1px solid var(--border); color: var(--text-2); white-space: nowrap;
  }
  .tag.own { background: var(--accent-tint); border-color: var(--accent-tint-border); color: var(--accent-text-2); }
  .add { display: flex; gap: var(--sp-2); flex-wrap: wrap; }
  .add input {
    min-width: 0; font: inherit; font-size: var(--fs-xs); color: var(--text);
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 8px 10px;
  }
  .add .ref { flex: 2 1 220px; font-family: var(--font-mono); }
  .add .key { flex: 1 1 140px; }
  .add input:focus { outline: none; border-color: var(--accent-border); }
</style>
