<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A private call node someone shared (`veydan://call-node/…`): where it
  is, and a way to connect it. The invitation in the link is spent by the
  exchange, so the card asks first.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { ask } from '$lib/core/ui/confirm.svelte';
  import type { LinkView } from '../../api';
  import { callNodeErrorText } from '../../net/errors';
  import { callNodesStore } from '../../net/callNodesStore.svelte';
  import { linkStore } from '../linkStore.svelte';

  interface Props { view: Extract<LinkView, { kind: 'call_node' }> }
  let { view }: Props = $props();

  /** Connected from this card, or known to be when the card was made. */
  let added = $state(false);
  /** The failure of connecting from this card; one elsewhere is not shown here. */
  let failure = $state('');
  const error = $derived(callNodeErrorText(failure, (k) => $t(k)));
  // An invitation is spent for nothing only when this device already holds
  // its own credentials there: a node of my own list (a shared key, or none)
  // still takes it.
  const known = $derived(
    added || view.added || (view.has_token ? callNodesStore.holdsCredentials(view.id) : callNodesStore.isMine(view.id)),
  );

  async function connect() {
    const yes = await ask({
      title: $t('msg_call_nodes_confirm_title'),
      message: $t('msg_call_nodes_confirm_text', { addr: view.addr }),
      confirmLabel: $t('msg_call_nodes_confirm_yes'),
      variant: 'primary',
    });
    if (!yes) return;
    failure = await callNodesStore.addFromCard(view.link);
    if (!failure) {
      added = true;
      linkStore.refresh();
    }
  }
</script>

<div class="box">
  <div class="top">
    <Icon name="network" size={22} />
    <div class="about">
      <span class="name">{$t('msg_call_node_card_title')}</span>
      <span class="sub"><code>{view.addr}</code></span>
    </div>
  </div>
  <p class="text">{$t(view.has_token ? 'msg_call_node_card_text' : 'msg_call_node_card_no_token')}</p>
  {#if error}<div class="error">{error}</div>{/if}
  <div class="row">
    {#if known}
      <span class="known"><Icon name="check" size={12} />{$t('msg_call_node_card_added')}</span>
    {:else if view.has_token}
      <button class="btn btn-primary btn-sm" disabled={callNodesStore.busy} onclick={connect}>
        <Icon name="plus" size={13} />{$t('msg_call_nodes_confirm_yes')}
      </button>
    {/if}
  </div>
</div>

<style>
  .box { display: flex; flex-direction: column; gap: var(--sp-2); }
  .top { display: flex; align-items: center; gap: var(--sp-3); min-width: 0; }
  .about { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-bold); }
  .sub { font-size: var(--fs-2xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub code { font-family: var(--font-mono); }
  .text { margin: 0; font-size: var(--fs-xs); color: var(--text-2); line-height: 1.4; }
  .row { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .known { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-2xs); color: var(--text-3); padding: 0 4px; }
  .error { font-size: var(--fs-xs); color: var(--danger-text); line-height: 1.4; }
</style>
