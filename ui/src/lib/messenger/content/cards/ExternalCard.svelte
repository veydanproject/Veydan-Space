<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  A link outside: where it really leads and what deserves a second look.
  The page itself is asked only when the button is pressed.
-->
<script lang="ts">
  import { t } from '$lib/core/i18n';
  import Icon from '$lib/core/Icon.svelte';
  import { messengerError } from '../../api';
  import { linkActions } from '../actions';
  import { canPreview, describeExternal } from '../host';
  import { linkStore } from '../linkStore.svelte';
  import type { ExternalUrl } from '../types';

  /** Refusals that have words of their own; any other is "did not answer". */
  const KNOWN = new Set(['preview_timeout', 'preview_empty', 'preview_not_a_page', 'preview_silent', 'preview_private_host', 'preview_too_many_redirects']);

  interface Props { url: ExternalUrl }
  let { url }: Props = $props();

  let error = $state('');

  const info = $derived(describeExternal(url));
  const risky = $derived(info.risks.some((r) => r !== 'port'));
  const shown = $derived(linkStore.preview(url));
  const preview = $derived(shown?.status === 'ready' ? shown.preview : null);

  async function open() {
    error = '';
    try { await linkActions.openExternal(url); }
    catch (e) { error = messengerError(e); }
  }
</script>

<div class="box">
  {#if preview?.image}
    <!-- The picture is inside the page already (`data:`): showing it asks nobody. -->
    <img class="cover" src={preview.image} alt="" />
  {/if}

  <div class="top">
    <span class="ico" class:risky><Icon name={risky ? 'alert-triangle' : info.secure ? 'globe' : 'lock-open'} size={16} /></span>
    <div class="about">
      <span class="host">{preview && preview.host !== info.host ? preview.host : info.host}</span>
      {#if info.path}<span class="path">{info.path}</span>{/if}
    </div>
  </div>

  {#if preview}
    {#if preview.title}<div class="title">{preview.title}</div>{/if}
    {#if preview.description}<div class="text">{preview.description}</div>{/if}
    {#if preview.host !== info.host}<div class="note">{$t('msg_card_link_moved', { host: info.host })}</div>{/if}
  {/if}

  {#each info.risks as r (r)}
    <div class="risk"><Icon name="alert-triangle" size={12} />{$t(`msg_card_link_risk_${r}` as 'msg_card_link_risk_insecure')}</div>
  {/each}
  {#if shown?.status === 'failed'}
    <div class="note">{$t((KNOWN.has(shown.code) ? `msg_card_${shown.code}` : 'msg_card_preview_failed') as 'msg_card_preview_failed')}</div>
  {/if}
  {#if error}<div class="risk">{error}</div>{/if}

  <div class="row">
    <button class="btn btn-ghost btn-sm" onclick={open}><Icon name="external-link" size={13} />{$t('msg_card_link_open')}</button>
    {#if preview}
      <button class="btn btn-ghost btn-sm" onclick={() => linkStore.hidePreview(url)}>{$t('msg_card_preview_hide')}</button>
    {:else if canPreview(url)}
      <button class="btn btn-ghost btn-sm" disabled={shown?.status === 'loading'} onclick={() => linkStore.askPreview(url)}
        title={$t('msg_card_preview_hint', { host: info.host })}>
        <Icon name={shown?.status === 'loading' ? 'loader' : 'eye'} size={13} />{$t('msg_card_preview_show')}
      </button>
    {/if}
  </div>
</div>

<style>
  .box { display: flex; flex-direction: column; gap: 6px; }
  .cover { display: block; width: 100%; max-height: 180px; object-fit: cover; border-radius: var(--radius-sm); background: var(--surface-3); }
  .top { display: flex; align-items: center; gap: var(--sp-2); min-width: 0; }
  .ico {
    width: 30px; height: 30px; flex-shrink: 0; border-radius: 50%; display: inline-flex; align-items: center; justify-content: center;
    background: var(--surface-3); color: var(--text-2);
  }
  .ico.risky { color: var(--warn-text); background: var(--warn-bg); }
  .about { display: flex; flex-direction: column; min-width: 0; }
  .host { font-size: var(--fs-xs); font-weight: var(--fw-bold); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .path { font-size: var(--fs-2xs); color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .title { font-size: var(--fs-sm); font-weight: var(--fw-semibold); line-height: 1.35; overflow-wrap: anywhere; }
  .text { font-size: var(--fs-xs); color: var(--text-2); line-height: 1.45; overflow-wrap: anywhere; display: -webkit-box; -webkit-line-clamp: 4; line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden; }
  .risk { display: flex; align-items: flex-start; gap: 5px; font-size: var(--fs-2xs); color: var(--warn-text); line-height: 1.4; }
  .risk :global(svg) { flex-shrink: 0; margin-top: 2px; }
  .note { font-size: var(--fs-2xs); color: var(--text-3); line-height: 1.4; }
  .row { display: flex; gap: 2px; flex-wrap: wrap; margin-left: -6px; }
</style>
