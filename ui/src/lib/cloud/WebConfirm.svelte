<!-- SPDX-FileCopyrightText: 2026 Veydan Project -->
<!-- SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1 -->

<!--
  «Удалить аккаунт Veydan Cloud?» asked by the web cabinet (cloud-api.md
  16.3, 16.5): the card `web.confirm` leads here with its values held to
  their shape (`webConfirmOf`). What goes away is said as in «Удалить
  аккаунт» of the app (DeleteAccount.svelte), with where the request came
  from; then the same guards — the secret of the lock and the word DELETE,
  checked again by the module — and `cloud_web_confirm`. The site deletes
  the account after it; this device unbinds on the card `account.deleted`.
  Nothing is sent by leaving: the request runs out in five minutes.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t, locale } from '$lib/core/i18n';
  import { api } from '$lib/core/api';
  import { asCloudError, webApi } from '$lib/cloud/api';
  import { deleteReady, emptyDeleteForm, errorKeyOf, type DeleteForm } from '$lib/cloud/account';
  import { CABINET_HOST, originLine, webConfirmOf } from '$lib/cloud/web';
  import Actions from '$lib/cloud/Actions.svelte';
  import Countdown from '$lib/cloud/Countdown.svelte';
  import Notice from '$lib/cloud/Notice.svelte';
  import StateView from '$lib/cloud/StateView.svelte';

  let { search = '', onleave }: { search?: string; onleave?: () => void } = $props();

  /** An action of the cabinet waits five minutes (cloud-api.md 16.2). */
  const ACTION_TTL_SECS = 300;

  const ask = $derived(webConfirmOf(search));
  let form = $state<DeleteForm>(emptyDeleteForm(false));
  let phase = $state<'ask' | 'sending' | 'done' | 'over'>('ask');
  let error = $state('');
  let now = $state(Date.now() / 1000);
  const ready = $derived(deleteReady(form));
  const origin = $derived(ask ? originLine(ask.country, ask.uaFamily, $t, $locale) : '');

  onMount(() => {
    void api.lock.status().then((s) => (form = emptyDeleteForm(s.enabled))).catch(() => {});
    const timer = setInterval(() => {
      now = Date.now() / 1000;
      if (phase === 'ask' && ask?.expiresAt && now >= ask.expiresAt) phase = 'over';
    }, 1000);
    if (ask?.expiresAt && now >= ask.expiresAt) phase = 'over';
    return () => clearInterval(timer);
  });

  async function confirm() {
    if (!ask || !ready || phase !== 'ask') return;
    error = '';
    phase = 'sending';
    try {
      await webApi.confirm(ask.id, form.word, form.lockOn ? form.secret : null);
      form.secret = '';
      phase = 'done';
    } catch (e) {
      const refusal = asCloudError(e);
      if (refusal.code === 'action_expired') {
        phase = 'over';
        return;
      }
      phase = 'ask';
      if (refusal.code === 'not_found') error = $t('cloud_web_confirm_not_found');
      else {
        const key = errorKeyOf(refusal.code);
        error = key ? $t(key) : refusal.message || refusal.code;
      }
    }
  }
</script>

{#if !ask}
  <StateView tone="warn" title={$t('cloud_web_confirm_not_found')}>
    {#snippet actions()}
      <Actions><button class="btn btn-primary" type="button" onclick={() => onleave?.()}>{$t('cloud_done')}</button></Actions>
    {/snippet}
  </StateView>
{:else if phase === 'done'}
  <StateView tone="ok" title={$t('cloud_web_confirm_done_title')}>
    <p>{$t('cloud_web_confirm_done_text', { host: CABINET_HOST })}</p>
    {#snippet actions()}
      <Actions><button class="btn btn-primary" type="button" onclick={() => onleave?.()}>{$t('cloud_done')}</button></Actions>
    {/snippet}
  </StateView>
{:else if phase === 'over'}
  <StateView tone="info" title={$t('cloud_card_web_confirm_expired')}>
    <p>{$t('cloud_web_confirm_over_text', { host: CABINET_HOST })}</p>
    {#snippet actions()}
      <Actions><button class="btn btn-primary" type="button" onclick={() => onleave?.()}>{$t('cloud_done')}</button></Actions>
    {/snippet}
  </StateView>
{:else}
  <div class="zone">
    <h2>{$t('cloud_web_confirm_title')}</h2>
    <Notice tone="warn">{$t('cloud_web_confirm_from', { host: CABINET_HOST, origin })}</Notice>
    {#if ask.expiresAt}
      <Countdown expiresAt={ask.expiresAt} total={ACTION_TTL_SECS} {now} label={(time) => $t('cloud_web_time_left', { time })} />
    {/if}
    <p class="text">{$t('cloud_delete_intro')}</p>
    <ul class="what">
      <li>{$t('cloud_delete_what_files')}</li>
      <li>{$t('cloud_delete_what_recovery')}</li>
      <li>{$t('cloud_delete_what_media')}</li>
      <li>{$t('cloud_delete_what_push')}</li>
      <li>{$t('cloud_delete_what_plan')}</li>
    </ul>
    <div class="notes">
      <p>{$t('cloud_delete_relays')}</p>
      <p>{$t('cloud_delete_payments')}</p>
      <p>{$t('cloud_web_confirm_not_me')}</p>
    </div>
    <form class="form" onsubmit={(e) => { e.preventDefault(); void confirm(); }}>
      {#if form.lockOn}
        <div class="form-group">
          <label for="cloud-web-lock">{$t('cloud_delete_lock')}</label>
          <input id="cloud-web-lock" type="password" bind:value={form.secret} autocomplete="current-password" />
        </div>
      {/if}
      <div class="form-group">
        <label for="cloud-web-word">{$t('cloud_delete_word')}</label>
        <input id="cloud-web-word" class="mono" type="text" bind:value={form.word} autocomplete="off" autocapitalize="characters" spellcheck="false" placeholder="DELETE" />
      </div>
      {#if error}<div class="error-msg" role="alert">{error}</div>{/if}
      <Actions>
        <button class="btn btn-ghost" type="button" disabled={phase === 'sending'} onclick={() => onleave?.()}>{$t('cloud_delete_cancel')}</button>
        <button class="btn btn-danger" type="submit" disabled={!ready || phase === 'sending'}>
          {phase === 'sending' ? $t('cloud_delete_running') : $t('cloud_web_confirm_delete')}
        </button>
      </Actions>
    </form>
  </div>
{/if}

<style>
  .zone { display: flex; flex-direction: column; gap: var(--sp-3); }
  h2 { margin: 0; font-size: var(--fs-md); font-weight: var(--fw-bold); color: var(--danger-text); }
  .text { margin: 0; font-size: var(--fs-base); color: var(--text); line-height: 1.5; }
  .what { margin: 0; padding-left: var(--sp-5); font-size: var(--fs-base); color: var(--text-body); line-height: 1.6; }
  .notes { display: flex; flex-direction: column; gap: var(--sp-1); }
  .notes p { margin: 0; font-size: var(--fs-sm); color: var(--text-3); line-height: 1.45; }
  .form { display: flex; flex-direction: column; gap: var(--sp-3); padding-top: var(--sp-3); border-top: 1px solid var(--border); }
</style>
