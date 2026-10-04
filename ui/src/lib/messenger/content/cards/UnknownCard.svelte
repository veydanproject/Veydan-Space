<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A link inside that leads nowhere here: written by a newer version, or
  broken. It is never opened as a link outside and never shown as a
  link that works.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import type { LinkView } from '../../api';
  import { linkActions } from '../actions';

  interface Props {
    view: Extract<LinkView, { kind: 'unknown' | 'invalid' }>;
    /** The link as the message has it. */
    text: string;
  }
  let { view, text }: Props = $props();

  let copied = $state(false);

  async function copy() {
    try {
      await linkActions.copy(text);
      copied = true;
      setTimeout(() => (copied = false), 1600);
    } catch { /* nothing to copy to */ }
  }
</script>

<div class="box">
  <div class="top">
    <span class="ico" class:bad={view.kind === 'invalid'}><Icon name={view.kind === 'unknown' ? 'info' : 'alert-triangle'} size={18} /></span>
    <div class="about">
      <span class="name">{$t(view.kind === 'unknown' ? 'msg_card_unknown_title' : 'msg_card_invalid_title')}</span>
      <span class="sub">
        {#if view.kind === 'unknown'}{$t('msg_card_unknown_text', { type: view.link_type })}
        {:else}{$t('msg_card_invalid_text')}{/if}
      </span>
    </div>
  </div>
  <button class="btn btn-ghost btn-sm" onclick={copy}><Icon name={copied ? 'check' : 'copy'} size={13} />{copied ? $t('msg_group_link_copied') : $t('msg_copy')}</button>
</div>

<style>
  .box { display: flex; flex-direction: column; gap: var(--sp-2); align-items: flex-start; }
  .top { display: flex; align-items: flex-start; gap: var(--sp-3); min-width: 0; }
  .ico {
    width: 38px; height: 38px; flex-shrink: 0; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center;
    background: var(--surface-3); color: var(--text-2);
  }
  .ico.bad { color: var(--warn-text); background: var(--warn-bg); }
  .about { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .name { font-size: var(--fs-sm); font-weight: var(--fw-bold); }
  .sub { font-size: var(--fs-xs); color: var(--text-2); line-height: 1.4; overflow-wrap: anywhere; }
</style>
