<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!-- A bridge someone shared: where it is, and a way to start using it. -->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import type { LinkView } from '../../api';
  import { netErrorText } from '../../net/errors';
  import { netStore } from '../../net/netStore.svelte';

  interface Props { view: Extract<LinkView, { kind: 'vlink' }> }
  let { view }: Props = $props();

  /** Added from this card, or known to be added when the card was made. */
  let added = $state(false);
  /** The failure of adding from this card; a failure elsewhere is not shown here. */
  let failure = $state('');
  const error = $derived(netErrorText(failure, $t));
  const known = $derived(added || view.added || Boolean(netStore.status?.private.some((b) => b.id === view.id)));

  async function add() {
    failure = await netStore.addBridgeFromCard(view.link);
    if (!failure) added = true;
  }
</script>

<div class="box">
  <div class="top">
    <Icon name="shield" size={22} />
    <div class="about">
      <span class="name">{$t('msg_bridge_card_title')}</span>
      <span class="sub"><code>{view.addr}</code></span>
    </div>
  </div>
  <p class="text">{$t('msg_bridge_card_text')}</p>
  {#if error}<div class="error">{error}</div>{/if}
  <div class="row">
    {#if known}
      <span class="known"><Icon name="check" size={12} />{$t('msg_bridge_card_added')}</span>
    {:else}
      <button class="btn btn-primary btn-sm" disabled={netStore.busy} onclick={add}>
        <Icon name="plus" size={13} />{$t('msg_bridge_add')}
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
